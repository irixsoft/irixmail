use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use serde::{Deserialize, Serialize};

use irixmail_core::{Error, Result};

use crate::rocksdb_store::SCHEMA_VERSION;
use crate::traits_store::Store;

pub const ARCHIVE_FORMAT: u32 = 1;

const MANIFEST: &str = "manifest.json";
const CONFIG: &str = "config.toml";
const SECRET_KEY: &str = "credential.key";
const DB: &str = "db";
const BLOBS: &str = "blobs";
const CERTS: &str = "certs";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: u32,
    pub created_at: u64,
    pub hostname: String,
    pub version: String,
    pub schema: u32,
}

#[derive(Clone, Debug)]
pub struct BackupPaths {
    pub config_file: PathBuf,
    pub db: PathBuf,
    pub blobs: PathBuf,
    pub secret_key: PathBuf,
    pub certs: PathBuf,
}

#[derive(Debug)]
pub struct Unpacked {
    pub manifest: Manifest,
    pub config_toml: Option<String>,
}

struct Staging(PathBuf);

impl Staging {
    fn next_to(db: &Path) -> Result<Self> {
        let parent = db
            .parent()
            .ok_or_else(|| Error::store("the database path has no parent directory"))?;
        let dir = parent.join(format!(
            ".backup-staging-{}-{}",
            std::process::id(),
            unix_now_nanos()
        ));
        fs::create_dir_all(&dir)?;
        Ok(Self(dir))
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub fn write_archive(
    paths: &BackupPaths,
    store: &dyn Store,
    hostname: &str,
    version: &str,
    out: impl Write,
) -> Result<Manifest> {
    let staging = Staging::next_to(&paths.db)?;
    let checkpoint = staging.0.join(DB);
    store.checkpoint(&checkpoint)?;
    let manifest = Manifest {
        format: ARCHIVE_FORMAT,
        created_at: unix_now(),
        hostname: hostname.to_string(),
        version: version.to_string(),
        schema: SCHEMA_VERSION,
    };
    let encoded = serde_json::to_vec_pretty(&manifest)
        .map_err(|err| Error::serialize(format!("could not encode the backup manifest: {err}")))?;

    let mut builder = tar::Builder::new(GzEncoder::new(out, Compression::fast()));
    builder.follow_symlinks(false);
    append_bytes(&mut builder, MANIFEST, &encoded)?;
    append_file_if_present(&mut builder, &paths.config_file, CONFIG)?;
    append_file_if_present(&mut builder, &paths.secret_key, SECRET_KEY)?;
    append_tree(&mut builder, &checkpoint, Path::new(DB), &|_| true)?;
    append_tree(&mut builder, &paths.blobs, Path::new(BLOBS), &|name| {
        !name.contains(".tmp.")
    })?;
    append_tree(&mut builder, &paths.certs, Path::new(CERTS), &|_| true)?;
    builder.into_inner()?.finish()?;
    Ok(manifest)
}

fn append_bytes<W: Write>(builder: &mut tar::Builder<W>, name: &str, bytes: &[u8]) -> Result<()> {
    let mut header = tar::Header::new_gnu();
    header.set_size(bytes.len() as u64);
    header.set_mode(0o644);
    header.set_mtime(unix_now());
    header.set_cksum();
    builder.append_data(&mut header, name, bytes)?;
    Ok(())
}

fn append_file_if_present<W: Write>(
    builder: &mut tar::Builder<W>,
    path: &Path,
    name: &str,
) -> Result<()> {
    if path.is_file() {
        builder.append_path_with_name(path, name)?;
    }
    Ok(())
}

fn append_tree<W: Write>(
    builder: &mut tar::Builder<W>,
    root: &Path,
    prefix: &Path,
    keep: &dyn Fn(&str) -> bool,
) -> Result<()> {
    if !root.is_dir() {
        return Ok(());
    }
    builder.append_dir(prefix, root)?;
    let mut entries: Vec<_> = fs::read_dir(root)?.collect::<std::io::Result<_>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry.file_name();
        let target = prefix.join(&name);
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            append_tree(builder, &entry.path(), &target, keep)?;
        } else if file_type.is_file() && keep(&name.to_string_lossy()) {
            builder.append_path_with_name(entry.path(), &target)?;
        }
    }
    Ok(())
}

fn open(archive: &Path) -> Result<tar::Archive<GzDecoder<File>>> {
    let file = File::open(archive)
        .map_err(|err| Error::store(format!("could not open {}: {err}", archive.display())))?;
    Ok(tar::Archive::new(GzDecoder::new(file)))
}

pub fn read_manifest(archive: &Path) -> Result<Manifest> {
    let mut reader = open(archive)?;
    for entry in reader.entries()? {
        let mut entry = entry?;
        if entry.path()?.as_ref() == Path::new(MANIFEST) {
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes)?;
            return serde_json::from_slice(&bytes).map_err(|err| {
                Error::invalid_input(format!("the backup manifest is unreadable: {err}"))
            });
        }
    }
    Err(Error::invalid_input(
        "the archive has no manifest.json; it is not an IRIXMAIL backup",
    ))
}

pub fn check_compatible(manifest: &Manifest) -> Result<()> {
    if manifest.format != ARCHIVE_FORMAT {
        return Err(Error::invalid_input(format!(
            "the backup uses archive format {} but this build reads format {ARCHIVE_FORMAT}",
            manifest.format
        )));
    }
    if manifest.schema > SCHEMA_VERSION {
        return Err(Error::invalid_input(format!(
            "the backup was made with store schema {} but this build supports {SCHEMA_VERSION}; update irixmail first",
            manifest.schema
        )));
    }
    Ok(())
}

pub fn unpack(archive: &Path, paths: &BackupPaths) -> Result<Unpacked> {
    let manifest = read_manifest(archive)?;
    check_compatible(&manifest)?;
    replace_dir(&paths.db)?;
    replace_dir(&paths.blobs)?;
    replace_dir(&paths.certs)?;
    let mut config_toml = None;
    let mut reader = open(archive)?;
    for entry in reader.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        let (first, rest) = split_first(&path)?;
        let destination = match first.as_str() {
            MANIFEST => continue,
            CONFIG => {
                let mut text = String::new();
                entry.read_to_string(&mut text)?;
                config_toml = Some(text);
                continue;
            }
            SECRET_KEY if rest.as_os_str().is_empty() => paths.secret_key.clone(),
            DB => paths.db.join(&rest),
            BLOBS => paths.blobs.join(&rest),
            CERTS => paths.certs.join(&rest),
            _ => continue,
        };
        let kind = entry.header().entry_type();
        if kind.is_dir() {
            fs::create_dir_all(&destination)?;
            continue;
        }
        if !kind.is_file() {
            continue;
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        let mode = entry.header().mode().unwrap_or(0o600) & 0o777;
        let mut file = File::create(&destination)?;
        std::io::copy(&mut entry, &mut file)?;
        fs::set_permissions(&destination, fs::Permissions::from_mode(mode))?;
    }
    Ok(Unpacked {
        manifest,
        config_toml,
    })
}

fn split_first(path: &Path) -> Result<(String, PathBuf)> {
    let mut components = path.components();
    let first = match components.next() {
        Some(Component::Normal(name)) => name.to_string_lossy().into_owned(),
        _ => return Err(Error::invalid_input(format!("unsafe path in archive: {}", path.display()))),
    };
    let mut rest = PathBuf::new();
    for component in components {
        match component {
            Component::Normal(name) => rest.push(name),
            Component::CurDir => {}
            _ => {
                return Err(Error::invalid_input(format!(
                    "unsafe path in archive: {}",
                    path.display()
                )))
            }
        }
    }
    Ok((first, rest))
}

fn replace_dir(dir: &Path) -> Result<()> {
    if dir.exists() {
        fs::remove_dir_all(dir)?;
    }
    fs::create_dir_all(dir)?;
    Ok(())
}

pub fn dir_has_entries(dir: &Path) -> bool {
    fs::read_dir(dir)
        .map(|mut entries| entries.next().is_some())
        .unwrap_or(false)
}

pub fn utc_parts(secs: u64) -> (i64, u32, u32, u32, u32) {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day, (rem / 3_600) as u32, ((rem % 3_600) / 60) as u32)
}

pub fn format_utc(secs: u64) -> String {
    let (year, month, day, hour, minute) = utc_parts(secs);
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02} UTC")
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

fn unix_now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::Subspace;
    use crate::rocksdb_store::RocksdbStore;

    struct Tree(PathBuf);

    impl Tree {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "irixmail-backup-{tag}-{}-{}",
                std::process::id(),
                unix_now_nanos()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn paths(&self) -> BackupPaths {
            BackupPaths {
                config_file: self.0.join("config.toml"),
                db: self.0.join("db"),
                blobs: self.0.join("blobs"),
                secret_key: self.0.join("credential.key"),
                certs: self.0.join("certs"),
            }
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write(path: &Path, bytes: &[u8], mode: u32) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    }

    fn key() -> Vec<u8> {
        vec![Subspace::Registry.as_byte(), 0x7f, 1]
    }

    #[test]
    fn an_archive_round_trips_the_whole_data_set() {
        let source = Tree::new("src");
        let paths = source.paths();
        write(&paths.config_file, b"[server]\nhostname = \"old.example.com\"\n", 0o640);
        write(&paths.secret_key, b"deadbeef", 0o600);
        write(&paths.certs.join("old.example.com.pem"), b"pem", 0o600);
        write(&paths.blobs.join("ab/cd/abcd0123"), b"blob", 0o644);
        write(&paths.blobs.join("ab/cd/abcd0123.tmp.7.1"), b"staging", 0o644);
        let store = RocksdbStore::open(&paths.db).unwrap();
        store.put(&key(), b"value").unwrap();

        let mut archive = Vec::new();
        let manifest =
            write_archive(&paths, &store, "old.example.com", "0.1.6", &mut archive).unwrap();
        drop(store);

        assert_eq!(manifest.format, ARCHIVE_FORMAT);
        assert_eq!(manifest.schema, SCHEMA_VERSION);
        assert_eq!(manifest.hostname, "old.example.com");
        assert!(archive.starts_with(&[0x1f, 0x8b]));
        let leftovers: Vec<_> = fs::read_dir(&source.0)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(".backup-staging"))
            .collect();
        assert!(leftovers.is_empty(), "the staging directory is removed");

        let file = source.0.join("backup.tar.gz");
        fs::write(&file, &archive).unwrap();
        assert_eq!(read_manifest(&file).unwrap(), manifest);

        let target = Tree::new("dst");
        let restored = target.paths();
        write(&restored.blobs.join("stale"), b"old", 0o644);
        let unpacked = unpack(&file, &restored).unwrap();
        assert_eq!(unpacked.manifest, manifest);
        assert_eq!(
            unpacked.config_toml.as_deref(),
            Some("[server]\nhostname = \"old.example.com\"\n")
        );
        assert!(!restored.config_file.exists());
        assert_eq!(fs::read(&restored.secret_key).unwrap(), b"deadbeef");
        assert_eq!(
            fs::metadata(&restored.secret_key).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(fs::read(restored.certs.join("old.example.com.pem")).unwrap(), b"pem");
        assert_eq!(fs::read(restored.blobs.join("ab/cd/abcd0123")).unwrap(), b"blob");
        assert!(!restored.blobs.join("ab/cd/abcd0123.tmp.7.1").exists());
        assert!(!restored.blobs.join("stale").exists());
        let reopened = RocksdbStore::open(&restored.db).unwrap();
        assert_eq!(reopened.get(&key()).unwrap().as_deref(), Some(&b"value"[..]));
    }

    #[test]
    fn a_backup_from_a_newer_schema_is_refused() {
        let tree = Tree::new("newer");
        let file = tree.0.join("newer.tar.gz");
        let manifest = Manifest {
            format: ARCHIVE_FORMAT,
            created_at: 0,
            hostname: "x".into(),
            version: "9.9.9".into(),
            schema: SCHEMA_VERSION + 1,
        };
        let mut builder = tar::Builder::new(GzEncoder::new(
            File::create(&file).unwrap(),
            Compression::fast(),
        ));
        append_bytes(&mut builder, MANIFEST, &serde_json::to_vec(&manifest).unwrap()).unwrap();
        builder.into_inner().unwrap().finish().unwrap();

        let error = unpack(&file, &tree.paths()).unwrap_err().to_string();
        assert!(error.contains("schema"), "{error}");
        assert!(!tree.paths().db.exists(), "nothing is touched before the check");
    }

    #[test]
    fn archive_paths_cannot_escape_their_target() {
        assert!(split_first(Path::new("../etc/passwd")).is_err());
        assert!(split_first(Path::new("db/../../x")).is_err());
        assert_eq!(
            split_first(Path::new("blobs/ab/cd")).unwrap(),
            ("blobs".to_string(), PathBuf::from("ab/cd"))
        );
    }

    #[test]
    fn utc_parts_match_the_calendar() {
        assert_eq!(utc_parts(0), (1970, 1, 1, 0, 0));
        assert_eq!(utc_parts(1_700_000_000), (2023, 11, 14, 22, 13));
        assert_eq!(format_utc(951_782_400), "2000-02-29 00:00 UTC");
    }
}
