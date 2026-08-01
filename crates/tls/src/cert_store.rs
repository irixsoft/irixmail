use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use pem::Pem;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};

use irixmail_core::{Error, Result};

pub struct CertMaterial {
    pub chain: Vec<CertificateDer<'static>>,
    pub key: PrivateKeyDer<'static>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CertSource {
    Acme,
    Uploaded,
    SelfSigned,
}

impl CertSource {
    fn as_str(self) -> &'static str {
        match self {
            CertSource::Acme => "acme",
            CertSource::Uploaded => "uploaded",
            CertSource::SelfSigned => "self-signed",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "acme" => Some(CertSource::Acme),
            "uploaded" => Some(CertSource::Uploaded),
            "self-signed" => Some(CertSource::SelfSigned),
            _ => None,
        }
    }
}

pub struct CertStore {
    dir: PathBuf,
}

impl CertStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn save(&self, name: &str, material: &CertMaterial, source: CertSource) -> Result<()> {
        fs::create_dir_all(&self.dir)?;
        let bundle = format!(
            "{}{}",
            encode_key(&material.key),
            encode_chain(&material.chain)
        );
        let temp = self.dir.join(format!(".{name}.pem.tmp"));
        let written = write_0600_synced(&temp, bundle.as_bytes())
            .and_then(|()| Ok(fs::rename(&temp, self.pem_path(name))?));
        if written.is_err() {
            let _ = fs::remove_file(&temp);
            return written;
        }
        fs::write(self.source_path(name), source.as_str())?;
        Ok(())
    }

    pub fn source(&self, name: &str) -> Option<CertSource> {
        let text = fs::read_to_string(self.source_path(name)).ok()?;
        CertSource::parse(&text)
    }

    pub fn load(&self, name: &str) -> Result<Option<CertMaterial>> {
        let pem_path = self.pem_path(name);
        if pem_path.exists() {
            return Ok(Some(CertMaterial {
                chain: read_chain(&pem_path)?,
                key: read_key(&pem_path)?,
            }));
        }
        let cert_path = self.cert_path(name);
        let key_path = self.key_path(name);
        if !cert_path.exists() || !key_path.exists() {
            return Ok(None);
        }
        Ok(Some(CertMaterial {
            chain: read_chain(&cert_path)?,
            key: read_key(&key_path)?,
        }))
    }

    pub fn delete(&self, name: &str) -> Result<()> {
        for path in [
            self.pem_path(name),
            self.cert_path(name),
            self.key_path(name),
            self.source_path(name),
        ] {
            if path.exists() {
                fs::remove_file(path)?;
            }
        }
        Ok(())
    }

    fn pem_path(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{name}.pem"))
    }

    fn cert_path(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{name}.crt"))
    }

    fn key_path(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{name}.key"))
    }

    fn source_path(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{name}.source"))
    }
}

fn encode_chain(chain: &[CertificateDer<'static>]) -> String {
    chain
        .iter()
        .map(|der| pem::encode(&Pem::new("CERTIFICATE", der.as_ref().to_vec())))
        .collect()
}

fn encode_key(key: &PrivateKeyDer<'static>) -> String {
    let label = match key {
        PrivateKeyDer::Pkcs1(_) => "RSA PRIVATE KEY",
        PrivateKeyDer::Sec1(_) => "EC PRIVATE KEY",
        _ => "PRIVATE KEY",
    };
    pem::encode(&Pem::new(label, key.secret_der().to_vec()))
}

fn read_chain(path: &Path) -> Result<Vec<CertificateDer<'static>>> {
    let mut reader = BufReader::new(File::open(path)?);
    let chain = rustls_pemfile::certs(&mut reader).collect::<std::result::Result<Vec<_>, _>>()?;
    if chain.is_empty() {
        return Err(Error::invalid_input(format!(
            "no certificates in {}",
            path.display()
        )));
    }
    Ok(chain)
}

fn read_key(path: &Path) -> Result<PrivateKeyDer<'static>> {
    let mut reader = BufReader::new(File::open(path)?);
    rustls_pemfile::private_key(&mut reader)?
        .ok_or_else(|| Error::invalid_input(format!("no private key in {}", path.display())))
}

fn write_0600_synced(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn sample() -> CertMaterial {
        let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        CertMaterial {
            chain: vec![certified.cert.der().clone()],
            key: PrivateKeyDer::Pkcs8(certified.key_pair.serialize_der().into()),
        }
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("irixmail-certs-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn save_records_the_certificate_source() {
        let dir = temp_dir("source");
        let store = CertStore::new(&dir);
        store.save("mail", &sample(), CertSource::Acme).unwrap();

        assert_eq!(store.source("mail"), Some(CertSource::Acme));

        store.save("mail", &sample(), CertSource::Uploaded).unwrap();
        assert_eq!(store.source("mail"), Some(CertSource::Uploaded));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_bundle_without_a_source_marker_has_no_source() {
        let dir = temp_dir("nosource");
        fs::create_dir_all(&dir).unwrap();
        let material = sample();
        fs::write(
            dir.join("mail.pem"),
            format!(
                "{}{}",
                encode_key(&material.key),
                encode_chain(&material.chain)
            ),
        )
        .unwrap();

        let store = CertStore::new(&dir);
        assert!(store.load("mail").unwrap().is_some());
        assert_eq!(store.source("mail"), None);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_then_load_round_trips_the_material_at_0600() {
        let dir = temp_dir("rt");
        let store = CertStore::new(&dir);
        let material = sample();
        store
            .save("mail", &material, CertSource::SelfSigned)
            .unwrap();

        let loaded = store.load("mail").unwrap().expect("material present");
        assert_eq!(loaded.chain, material.chain);
        assert_eq!(loaded.key.secret_der(), material.key.secret_der());

        let mode = fs::metadata(dir.join("mail.pem"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn loading_absent_material_is_none() {
        let dir = temp_dir("none");
        assert!(CertStore::new(&dir).load("missing").unwrap().is_none());
    }

    #[test]
    fn save_publishes_one_atomic_bundle_file() {
        let dir = temp_dir("bundle");
        let store = CertStore::new(&dir);
        store
            .save("mail", &sample(), CertSource::SelfSigned)
            .unwrap();

        assert!(dir.join("mail.pem").exists());
        assert!(!dir.join("mail.crt").exists());
        assert!(!dir.join("mail.key").exists());
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().contains("tmp"))
            .collect();
        assert!(leftovers.is_empty());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn legacy_split_cert_and_key_files_still_load() {
        let dir = temp_dir("legacy");
        fs::create_dir_all(&dir).unwrap();
        let material = sample();
        fs::write(dir.join("mail.crt"), encode_chain(&material.chain)).unwrap();
        fs::write(dir.join("mail.key"), encode_key(&material.key)).unwrap();

        let loaded = CertStore::new(&dir)
            .load("mail")
            .unwrap()
            .expect("material present");
        assert_eq!(loaded.chain, material.chain);
        assert_eq!(loaded.key.secret_der(), material.key.secret_der());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_removes_the_bundle_and_legacy_files() {
        let dir = temp_dir("delete");
        let store = CertStore::new(&dir);
        let material = sample();
        store
            .save("mail", &material, CertSource::SelfSigned)
            .unwrap();
        fs::write(dir.join("mail.crt"), encode_chain(&material.chain)).unwrap();
        fs::write(dir.join("mail.key"), encode_key(&material.key)).unwrap();

        store.delete("mail").unwrap();
        assert!(store.load("mail").unwrap().is_none());
        assert_eq!(store.source("mail"), None);
        assert!(!dir.join("mail.pem").exists());
        assert!(!dir.join("mail.source").exists());
        assert!(!dir.join("mail.crt").exists());
        assert!(!dir.join("mail.key").exists());

        let _ = fs::remove_dir_all(&dir);
    }
}
