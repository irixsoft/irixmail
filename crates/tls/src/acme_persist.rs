use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;

use irixmail_core::Result;

const FILE_NAME: &str = "acme-account.json";

#[derive(Clone)]
pub struct AcmePersist {
    dir: PathBuf,
}

impl AcmePersist {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn save(&self, serialized: &str) -> Result<()> {
        fs::create_dir_all(&self.dir)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(self.dir.join(FILE_NAME))?;
        file.write_all(serialized.as_bytes())?;
        Ok(())
    }

    pub fn load(&self) -> Result<Option<String>> {
        let path = self.dir.join(FILE_NAME);
        if !path.exists() {
            return Ok(None);
        }
        Ok(Some(fs::read_to_string(path)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("irixmail-acme-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn credentials_round_trip_at_0600() {
        let dir = temp_dir("rt");
        let persist = AcmePersist::new(&dir);
        persist.save(r#"{"id":"acct"}"#).unwrap();
        assert_eq!(persist.load().unwrap().as_deref(), Some(r#"{"id":"acct"}"#));
        let mode = fs::metadata(dir.join(FILE_NAME))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn absent_credentials_load_as_none() {
        let dir = temp_dir("none");
        assert_eq!(AcmePersist::new(&dir).load().unwrap(), None);
    }
}
