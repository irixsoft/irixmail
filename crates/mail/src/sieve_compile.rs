use irixmail_core::{Error, Result};
use irixmail_directory::StoredScript;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct RuleSet {
    pub rules: Vec<Rule>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub name: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub match_type: MatchType,
    #[serde(default)]
    pub conditions: Vec<Condition>,
    pub actions: Vec<Action>,
}

fn default_enabled() -> bool {
    true
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MatchType {
    #[default]
    All,
    Any,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Condition {
    pub field: Field,
    pub comparator: Comparator,
    pub value: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    From,
    To,
    Cc,
    Subject,
    Header { name: String },
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Comparator {
    Contains,
    Is,
    Matches,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    FileInto { mailbox: String },
    Redirect { address: String },
    Discard,
    Keep,
    AddFlag { flag: String },
    Stop,
}

#[derive(Debug, Clone)]
pub struct CompiledScript {
    pub source: String,
    pub script: irixmail_sieve::Script,
}

pub fn compile_rules(rule_set: &RuleSet) -> Result<CompiledScript> {
    let source = emit_script(rule_set);
    compile_source(&source)
}

pub fn compile_source(source: &str) -> Result<CompiledScript> {
    let script = irixmail_sieve::Compiler::new()
        .compile(source)
        .map_err(|err| Error::invalid_input(format!("filter rules are not valid: {err}")))?;
    Ok(CompiledScript {
        source: source.to_string(),
        script,
    })
}

// Fail-open: a malformed rule or failed compile yields None and mail lands in the inbox.
pub fn compile_active_script(scripts: &[StoredScript]) -> Option<CompiledScript> {
    compile_stored_script(scripts.iter().find(|script| script.active)?)
}

pub fn compile_stored_script(script: &StoredScript) -> Option<CompiledScript> {
    match &script.rules {
        Some(rules) => {
            let rule_set = stored_rule_set(rules);
            if rule_set.rules.is_empty() {
                return None;
            }
            compile_rules(&rule_set).ok()
        }
        None if !script.source.trim().is_empty() => compile_source(&script.source).ok(),
        None => None,
    }
}

pub fn script_source(script: &StoredScript) -> String {
    if !script.source.is_empty() {
        return script.source.clone();
    }
    match &script.rules {
        Some(rules) => emit_script(&stored_rule_set(rules)),
        None => String::new(),
    }
}

pub fn stored_rule_set(rules: &Value) -> RuleSet {
    RuleSet {
        rules: rules
            .as_array()
            .map(|rules| rules.iter().filter_map(stored_rule).collect())
            .unwrap_or_default(),
    }
}

fn stored_rule(value: &Value) -> Option<Rule> {
    let field = match value.get("field")?.as_str()? {
        "from" => Field::From,
        "to" => Field::To,
        "subject" => Field::Subject,
        _ => return None,
    };
    let comparator = match value.get("operator")?.as_str()? {
        "contains" => Comparator::Contains,
        "is" => Comparator::Is,
        _ => return None,
    };
    let pattern = value.get("value")?.as_str()?.to_string();
    let target = value
        .get("target")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let action = match value.get("action")?.as_str()? {
        "fileinto" => Action::FileInto { mailbox: target },
        "forward" => Action::Redirect { address: target },
        "discard" => Action::Discard,
        "markRead" => Action::AddFlag {
            flag: "\\Seen".to_string(),
        },
        _ => return None,
    };
    Some(Rule {
        name: value
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        enabled: true,
        match_type: MatchType::All,
        conditions: vec![Condition {
            field,
            comparator,
            value: pattern,
        }],
        actions: vec![action],
    })
}

pub fn emit_script(rule_set: &RuleSet) -> String {
    let mut out = String::new();
    out.push_str("require [\"fileinto\", \"imap4flags\"];\r\n");

    for rule in rule_set.rules.iter().filter(|rule| rule.enabled) {
        emit_rule(&mut out, rule);
    }

    out
}

fn emit_rule(out: &mut String, rule: &Rule) {
    if rule.conditions.is_empty() {
        for action in &rule.actions {
            emit_action(out, action);
        }
        return;
    }

    out.push_str("if ");
    out.push_str(match rule.match_type {
        MatchType::All => "allof (",
        MatchType::Any => "anyof (",
    });
    for (index, condition) in rule.conditions.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        emit_condition(out, condition);
    }
    out.push_str(") {\r\n");
    for action in &rule.actions {
        out.push('\t');
        emit_action(out, action);
    }
    out.push_str("}\r\n");
}

fn emit_condition(out: &mut String, condition: &Condition) {
    out.push_str("header ");
    out.push_str(match condition.comparator {
        Comparator::Contains => ":contains ",
        Comparator::Is => ":is ",
        Comparator::Matches => ":matches ",
    });
    push_quoted(out, field_header(&condition.field));
    out.push(' ');
    push_quoted(out, &condition.value);
}

fn emit_action(out: &mut String, action: &Action) {
    match action {
        Action::FileInto { mailbox } => {
            out.push_str("fileinto ");
            push_quoted(out, mailbox);
            out.push(';');
        }
        Action::Redirect { address } => {
            out.push_str("redirect ");
            push_quoted(out, address);
            out.push(';');
        }
        Action::Discard => out.push_str("discard;"),
        Action::Keep => out.push_str("keep;"),
        Action::AddFlag { flag } => {
            out.push_str("addflag ");
            push_quoted(out, flag);
            out.push(';');
        }
        Action::Stop => out.push_str("stop;"),
    }
    out.push_str("\r\n");
}

fn field_header(field: &Field) -> &str {
    match field {
        Field::From => "From",
        Field::To => "To",
        Field::Cc => "Cc",
        Field::Subject => "Subject",
        Field::Header { name } => name,
    }
}

// Control bytes become spaces so a rule value can never inject a second Sieve statement.
fn push_quoted(out: &mut String, value: &str) {
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' | '\\' => {
                out.push('\\');
                out.push(ch);
            }
            ch if (ch as u32) < 0x20 || ch == '\u{7f}' => out.push(' '),
            _ => out.push(ch),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rule(conditions: Vec<Condition>, actions: Vec<Action>) -> Rule {
        Rule {
            name: "test".to_string(),
            enabled: true,
            match_type: MatchType::All,
            conditions,
            actions,
        }
    }

    fn stored(source: &str, rules: Option<Value>) -> StoredScript {
        StoredScript {
            id: "1".to_string(),
            name: "filters".to_string(),
            source: source.to_string(),
            rules,
            active: true,
        }
    }

    #[test]
    fn empty_rule_set_compiles_to_a_bare_require() {
        let compiled = compile_rules(&RuleSet::default()).unwrap();
        assert!(compiled.source.starts_with("require ["));
        assert!(!compiled.source.contains("if "));
    }

    #[test]
    fn a_condition_becomes_a_guarded_fileinto() {
        let rules = RuleSet {
            rules: vec![rule(
                vec![Condition {
                    field: Field::From,
                    comparator: Comparator::Contains,
                    value: "newsletter@example.com".to_string(),
                }],
                vec![Action::FileInto {
                    mailbox: "Newsletters".to_string(),
                }],
            )],
        };
        let compiled = compile_rules(&rules).unwrap();
        assert!(compiled
            .source
            .contains("if allof (header :contains \"From\" \"newsletter@example.com\")"));
        assert!(compiled.source.contains("fileinto \"Newsletters\";"));
    }

    #[test]
    fn any_match_emits_anyof_with_multiple_conditions() {
        let rules = RuleSet {
            rules: vec![Rule {
                match_type: MatchType::Any,
                ..rule(
                    vec![
                        Condition {
                            field: Field::Subject,
                            comparator: Comparator::Is,
                            value: "Sale".to_string(),
                        },
                        Condition {
                            field: Field::Header {
                                name: "List-Id".to_string(),
                            },
                            comparator: Comparator::Matches,
                            value: "*deals*".to_string(),
                        },
                    ],
                    vec![Action::Discard, Action::Stop],
                )
            }],
        };
        let compiled = compile_rules(&rules).unwrap();
        assert!(compiled.source.contains("anyof ("));
        assert!(compiled
            .source
            .contains("header :is \"Subject\" \"Sale\", header :matches \"List-Id\" \"*deals*\""));
        assert!(compiled.source.contains("discard;"));
        assert!(compiled.source.contains("stop;"));
    }

    #[test]
    fn a_rule_with_no_conditions_acts_unconditionally() {
        let rules = RuleSet {
            rules: vec![rule(
                Vec::new(),
                vec![Action::AddFlag {
                    flag: "\\Flagged".to_string(),
                }],
            )],
        };
        let compiled = compile_rules(&rules).unwrap();
        assert!(!compiled.source.contains("if "));
        assert!(compiled.source.contains("addflag \"\\\\Flagged\";"));
    }

    #[test]
    fn disabled_rules_are_not_emitted() {
        let rules = RuleSet {
            rules: vec![Rule {
                enabled: false,
                ..rule(
                    Vec::new(),
                    vec![Action::FileInto {
                        mailbox: "Spam".to_string(),
                    }],
                )
            }],
        };
        let compiled = compile_rules(&rules).unwrap();
        assert!(!compiled.source.contains("fileinto \"Spam\";"));
    }

    #[test]
    fn quotes_and_backslashes_in_values_are_escaped() {
        let rules = RuleSet {
            rules: vec![rule(
                vec![Condition {
                    field: Field::Subject,
                    comparator: Comparator::Contains,
                    value: "a\"b\\c".to_string(),
                }],
                vec![Action::Keep],
            )],
        };
        let compiled = compile_rules(&rules).unwrap();
        assert!(compiled.source.contains("\"a\\\"b\\\\c\""));
    }

    #[test]
    fn control_bytes_in_values_become_spaces() {
        let rules = RuleSet {
            rules: vec![rule(
                vec![Condition {
                    field: Field::Subject,
                    comparator: Comparator::Contains,
                    value: "a\r\nfileinto \"X\";\x07b".to_string(),
                }],
                vec![Action::Keep],
            )],
        };
        let compiled = compile_rules(&rules).unwrap();
        assert!(compiled.source.contains("\"a  fileinto \\\"X\\\"; b\""));
    }

    #[test]
    fn redirect_action_compiles() {
        let rules = RuleSet {
            rules: vec![rule(
                Vec::new(),
                vec![Action::Redirect {
                    address: "elsewhere@example.org".to_string(),
                }],
            )],
        };
        let compiled = compile_rules(&rules).unwrap();
        assert!(compiled
            .source
            .contains("redirect \"elsewhere@example.org\";"));
    }

    #[test]
    fn rule_set_round_trips_through_serde_json() {
        let rules = RuleSet {
            rules: vec![rule(
                vec![Condition {
                    field: Field::To,
                    comparator: Comparator::Is,
                    value: "me@example.com".to_string(),
                }],
                vec![Action::FileInto {
                    mailbox: "Direct".to_string(),
                }],
            )],
        };
        let json = serde_json::to_string(&rules).unwrap();
        let back: RuleSet = serde_json::from_str(&json).unwrap();
        assert_eq!(rules, back);
    }

    #[test]
    fn only_the_active_stored_script_is_compiled() {
        let inactive = StoredScript {
            active: false,
            ..stored(
                "",
                Some(json!([{"id": "r1", "field": "from", "operator": "is",
                "value": "a@b.example", "action": "discard", "target": ""}])),
            )
        };
        assert!(compile_active_script(std::slice::from_ref(&inactive)).is_none());
        let active = StoredScript {
            active: true,
            ..inactive
        };
        assert!(compile_active_script(&[active]).is_some());
    }

    #[test]
    fn a_stored_script_with_rules_compiles_from_its_rules() {
        let script = stored(
            "discard;",
            Some(
                json!([{"id": "r1", "field": "subject", "operator": "contains",
                "value": "receipt", "action": "fileinto", "target": "Receipts"}]),
            ),
        );
        let compiled = compile_stored_script(&script).unwrap();
        assert!(compiled.source.contains("fileinto \"Receipts\";"));
        assert!(!compiled.source.contains("discard"));
    }

    #[test]
    fn a_stored_script_without_rules_compiles_its_raw_source() {
        let script = stored("require \"fileinto\";\nfileinto \"Archive\";\n", None);
        let compiled = compile_stored_script(&script).unwrap();
        assert_eq!(compiled.source, script.source);
        assert!(compile_stored_script(&stored("  ", None)).is_none());
    }

    #[test]
    fn malformed_rules_and_sources_fail_open() {
        assert!(compile_stored_script(&stored(
            "",
            Some(json!([{"id": "r1",
            "field": "cc", "operator": "contains", "value": "x", "action": "fileinto",
            "target": "X"}]))
        ))
        .is_none());
        assert!(compile_stored_script(&stored("not a sieve script", None)).is_none());
        assert!(compile_stored_script(&stored("", Some(json!("not an array")))).is_none());
    }

    #[test]
    fn script_source_prefers_stored_source_and_falls_back_to_rules() {
        let raw = stored("keep;", None);
        assert_eq!(script_source(&raw), "keep;");
        let legacy = stored(
            "",
            Some(json!([{"id": "r1", "field": "from", "operator": "is",
                "value": "a@b.example", "action": "discard", "target": ""}])),
        );
        assert!(script_source(&legacy).contains("discard;"));
        assert_eq!(script_source(&stored("", None)), "");
    }
}
