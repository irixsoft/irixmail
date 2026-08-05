use std::sync::Arc;

use serde_json::{json, Value};

use irixmail_core::{Error, IdGenerator, Result};
use irixmail_store::{Store, Subspace, WriteOp};

const TAG_SIEVE_SCRIPT: u8 = 0x28;

#[derive(Debug, Clone, PartialEq)]
pub struct StoredScript {
    pub id: String,
    pub name: String,
    pub source: String,
    pub rules: Option<Value>,
    pub active: bool,
}

#[derive(Clone)]
pub struct SieveScriptRegistry {
    store: Arc<dyn Store>,
    ids: Arc<IdGenerator>,
}

impl SieveScriptRegistry {
    pub fn new(store: Arc<dyn Store>, ids: Arc<IdGenerator>) -> Self {
        Self { store, ids }
    }

    pub fn list(&self, account_id: u64) -> Result<Vec<StoredScript>> {
        let entries: Vec<Value> = match self.store.get(&record_key(account_id))? {
            Some(bytes) => serde_json::from_slice(&bytes).map_err(|err| {
                Error::serialize(format!("could not decode sieve scripts: {err}"))
            })?,
            None => return Ok(Vec::new()),
        };
        let legacy = entries.iter().all(|entry| entry.get("active").is_none());
        Ok(entries
            .iter()
            .enumerate()
            .map(|(index, entry)| StoredScript {
                id: entry
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                name: entry
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                source: entry
                    .get("source")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                rules: entry.get("rules").cloned(),
                active: entry
                    .get("active")
                    .and_then(Value::as_bool)
                    .unwrap_or(legacy && index == 0),
            })
            .collect())
    }

    pub fn get_by_name(&self, account_id: u64, name: &str) -> Result<Option<StoredScript>> {
        Ok(self
            .list(account_id)?
            .into_iter()
            .find(|script| script.name == name))
    }

    pub fn active_script(&self, account_id: u64) -> Result<Option<StoredScript>> {
        Ok(self
            .list(account_id)?
            .into_iter()
            .find(|script| script.active))
    }

    pub fn create(
        &self,
        account_id: u64,
        name: &str,
        source: &str,
        rules: Option<Value>,
    ) -> Result<StoredScript> {
        let mut scripts = self.list(account_id)?;
        if scripts.iter().any(|script| script.name == name) {
            return Err(Error::invalid_input(format!(
                "a script named \"{name}\" already exists"
            )));
        }
        let script = StoredScript {
            id: self.ids.generate().to_string(),
            name: name.to_string(),
            source: source.to_string(),
            rules,
            active: scripts.is_empty(),
        };
        scripts.push(script.clone());
        self.write(account_id, &scripts)?;
        Ok(script)
    }

    pub fn update(
        &self,
        account_id: u64,
        id: &str,
        name: Option<&str>,
        source: Option<&str>,
        rules: Option<Option<Value>>,
    ) -> Result<bool> {
        let mut scripts = self.list(account_id)?;
        if let Some(new_name) = name {
            if scripts
                .iter()
                .any(|script| script.name == new_name && script.id != id)
            {
                return Err(Error::invalid_input(format!(
                    "a script named \"{new_name}\" already exists"
                )));
            }
        }
        let Some(script) = scripts.iter_mut().find(|script| script.id == id) else {
            return Ok(false);
        };
        if let Some(name) = name {
            script.name = name.to_string();
        }
        if let Some(source) = source {
            script.source = source.to_string();
        }
        if let Some(rules) = rules {
            script.rules = rules;
        }
        self.write(account_id, &scripts)?;
        Ok(true)
    }

    pub fn set_active(&self, account_id: u64, id: Option<&str>) -> Result<bool> {
        let mut scripts = self.list(account_id)?;
        if let Some(id) = id {
            if !scripts.iter().any(|script| script.id == id) {
                return Ok(false);
            }
        }
        for script in &mut scripts {
            script.active = Some(script.id.as_str()) == id;
        }
        self.write(account_id, &scripts)?;
        Ok(true)
    }

    pub fn destroy(&self, account_id: u64, id: &str) -> Result<bool> {
        let mut scripts = self.list(account_id)?;
        let before = scripts.len();
        scripts.retain(|script| script.id != id);
        if scripts.len() == before {
            return Ok(false);
        }
        self.write(account_id, &scripts)?;
        Ok(true)
    }

    fn write(&self, account_id: u64, scripts: &[StoredScript]) -> Result<()> {
        let key = record_key(account_id);
        let op = if scripts.is_empty() {
            WriteOp::Delete { key }
        } else {
            let entries: Vec<Value> = scripts.iter().map(encode_script).collect();
            let value = serde_json::to_vec(&entries).map_err(|err| {
                Error::serialize(format!("could not encode sieve scripts: {err}"))
            })?;
            WriteOp::Set { key, value }
        };
        self.store.batch(&[op])
    }
}

fn encode_script(script: &StoredScript) -> Value {
    let mut entry = json!({
        "id": script.id,
        "name": script.name,
        "source": script.source,
        "active": script.active,
    });
    if let Some(rules) = &script.rules {
        entry["rules"] = rules.clone();
    }
    entry
}

fn record_key(account_id: u64) -> Vec<u8> {
    let mut key = Vec::with_capacity(2 + std::mem::size_of::<u64>());
    key.push(Subspace::Registry.as_byte());
    key.push(TAG_SIEVE_SCRIPT);
    key.extend_from_slice(&account_id.to_be_bytes());
    key
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use irixmail_store::{Flow, KeyPrefix};

    #[derive(Default)]
    struct MemStore {
        map: Mutex<BTreeMap<Vec<u8>, Vec<u8>>>,
    }

    impl Store for MemStore {
        fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
            Ok(self.map.lock().unwrap().get(key).cloned())
        }

        fn put(&self, key: &[u8], value: &[u8]) -> Result<()> {
            self.map
                .lock()
                .unwrap()
                .insert(key.to_vec(), value.to_vec());
            Ok(())
        }

        fn delete(&self, key: &[u8]) -> Result<()> {
            self.map.lock().unwrap().remove(key);
            Ok(())
        }

        fn iterate(
            &self,
            prefix: &KeyPrefix,
            visit: &mut dyn FnMut(&[u8], &[u8]) -> Result<Flow>,
        ) -> Result<()> {
            let bound = prefix.encode();
            let map = self.map.lock().unwrap();
            for (key, value) in map.iter() {
                if !key.starts_with(&bound) {
                    continue;
                }
                if visit(key, value)? == Flow::Stop {
                    break;
                }
            }
            Ok(())
        }

        fn batch(&self, ops: &[WriteOp]) -> Result<()> {
            let mut map = self.map.lock().unwrap();
            for op in ops {
                match op {
                    WriteOp::Set { key, value } => {
                        map.insert(key.clone(), value.clone());
                    }
                    WriteOp::Delete { key } => {
                        map.remove(key);
                    }
                    WriteOp::Add { .. } => unreachable!("the sieve registry does not use counters"),
                }
            }
            Ok(())
        }

        fn add_and_get(&self, _key: &[u8], _by: i64) -> Result<i64> {
            unreachable!("the sieve registry does not use counters")
        }

        fn counter(&self, _key: &[u8]) -> Result<i64> {
            unreachable!("the sieve registry does not use counters")
        }
    }

    fn registry() -> SieveScriptRegistry {
        SieveScriptRegistry::new(Arc::new(MemStore::default()), Arc::new(IdGenerator::new(0)))
    }

    #[test]
    fn an_account_starts_with_no_scripts() {
        assert!(registry().list(1).unwrap().is_empty());
    }

    #[test]
    fn a_script_is_created_updated_and_destroyed() {
        let registry = registry();
        let script = registry
            .create(1, "filters", "", Some(json!([{"id": "r1"}])))
            .unwrap();
        let listed = registry.list(1).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "filters");

        assert!(registry
            .update(1, &script.id, Some("renamed"), None, None)
            .unwrap());
        assert_eq!(registry.list(1).unwrap()[0].name, "renamed");

        assert!(registry.destroy(1, &script.id).unwrap());
        assert!(registry.list(1).unwrap().is_empty());
    }

    #[test]
    fn updating_or_destroying_an_unknown_id_reports_no_change() {
        let registry = registry();
        assert!(!registry
            .update(1, "missing", Some("x"), None, None)
            .unwrap());
        assert!(!registry.destroy(1, "missing").unwrap());
    }

    #[test]
    fn the_first_script_is_created_active_and_later_ones_are_not() {
        let registry = registry();
        let first = registry.create(1, "one", "keep;", None).unwrap();
        let second = registry.create(1, "two", "keep;", None).unwrap();
        assert!(first.active);
        assert!(!second.active);
        assert_eq!(registry.active_script(1).unwrap().unwrap().id, first.id);
    }

    #[test]
    fn activating_a_script_deactivates_the_others() {
        let registry = registry();
        let first = registry.create(1, "one", "keep;", None).unwrap();
        let second = registry.create(1, "two", "keep;", None).unwrap();
        assert!(registry.set_active(1, Some(&second.id)).unwrap());
        let listed = registry.list(1).unwrap();
        assert!(!listed[0].active);
        assert!(listed[1].active);
        assert!(registry.set_active(1, None).unwrap());
        assert!(registry.active_script(1).unwrap().is_none());
        assert!(!registry.set_active(1, Some("missing")).unwrap());
        let _ = first;
    }

    #[test]
    fn duplicate_script_names_are_rejected() {
        let registry = registry();
        let first = registry.create(1, "filters", "", None).unwrap();
        assert!(registry.create(1, "filters", "", None).is_err());
        let second = registry.create(1, "other", "", None).unwrap();
        assert!(registry
            .update(1, &second.id, Some("filters"), None, None)
            .is_err());
        assert!(registry
            .update(1, &first.id, Some("filters"), None, None)
            .unwrap());
    }

    #[test]
    fn legacy_entries_without_source_or_active_migrate_on_read() {
        let registry = registry();
        let legacy = serde_json::to_vec(&json!([
            {"id": "11", "name": "filters", "rules": [{"id": "r1", "field": "from"}]},
            {"id": "22", "name": "extra", "rules": []},
        ]))
        .unwrap();
        registry.store.put(&record_key(1), &legacy).unwrap();

        let listed = registry.list(1).unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].id, "11");
        assert_eq!(listed[0].source, "");
        assert!(listed[0].active);
        assert!(!listed[1].active);
        assert_eq!(
            listed[0].rules,
            Some(json!([{"id": "r1", "field": "from"}]))
        );

        assert!(registry.update(1, "11", None, Some("keep;"), None).unwrap());
        let migrated = registry.list(1).unwrap();
        assert_eq!(migrated[0].source, "keep;");
        assert!(migrated[0].active);
    }

    #[test]
    fn a_cleared_rules_sidecar_stays_cleared() {
        let registry = registry();
        let script = registry
            .create(1, "filters", "keep;", Some(json!([])))
            .unwrap();
        assert!(registry
            .update(1, &script.id, None, Some("discard;"), Some(None))
            .unwrap());
        let listed = registry.list(1).unwrap();
        assert_eq!(listed[0].rules, None);
        assert_eq!(listed[0].source, "discard;");
    }

    #[test]
    fn scripts_are_looked_up_by_exact_name() {
        let registry = registry();
        registry.create(1, "Filters", "keep;", None).unwrap();
        assert!(registry.get_by_name(1, "Filters").unwrap().is_some());
        assert!(registry.get_by_name(1, "filters").unwrap().is_none());
    }

    #[test]
    fn destroying_the_last_script_removes_the_record_entirely() {
        let registry = registry();
        let script = registry.create(1, "filters", "keep;", None).unwrap();
        assert!(registry.destroy(1, &script.id).unwrap());
        assert!(registry.store.get(&record_key(1)).unwrap().is_none());
    }
}
