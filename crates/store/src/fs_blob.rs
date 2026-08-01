use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use irixmail_core::{Error, Result};

use crate::traits_blob::{BlobHash, BlobStore};

static STAGING_SEQUENCE: AtomicU64 = AtomicU64::new(0);

const FANOUT_LEVELS: usize = 2;

pub struct FsBlobStore {
    root: PathBuf,
}

impl FsBlobStore {
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root).map_err(|err| {
            Error::store(format!(
                "failed to create blob directory {}: {err}",
                root.display()
            ))
        })?;
        Ok(Self { root })
    }

    fn digest(bytes: &[u8]) -> BlobHash {
        BlobHash::from_bytes(blake3::hash(bytes).as_bytes().to_vec())
    }

    fn blob_path(&self, hash: &BlobHash) -> PathBuf {
        let hex = hash.to_hex();
        let mut path = self.root.clone();
        for level in 0..FANOUT_LEVELS {
            let start = level * 2;
            let Some(segment) = hex.get(start..start + 2) else {
                break;
            };
            path.push(segment);
        }
        path.push(hex);
        path
    }

    fn prune_empty_shards(&self, blob_path: &Path) {
        let mut dir = blob_path.parent();
        while let Some(current) = dir {
            if current == self.root {
                break;
            }
            if fs::remove_dir(current).is_err() {
                break;
            }
            dir = current.parent();
        }
    }
}

impl BlobStore for FsBlobStore {
    fn get(&self, hash: &BlobHash, range: Range<usize>) -> Result<Option<Vec<u8>>> {
        let path = self.blob_path(hash);
        let mut file = match File::open(&path) {
            Ok(file) => file,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => {
                return Err(Error::store(format!("failed to open blob {hash}: {err}")));
            }
        };

        let len = file
            .metadata()
            .map_err(|err| Error::store(format!("failed to stat blob {hash}: {err}")))?
            .len() as usize;

        let start = range.start.min(len);
        let end = range.end.min(len).max(start);
        let mut buf = vec![0u8; end - start];

        if start > 0 {
            file.seek(SeekFrom::Start(start as u64))
                .map_err(|err| Error::store(format!("failed to seek blob {hash}: {err}")))?;
        }
        file.read_exact(&mut buf)
            .map_err(|err| Error::store(format!("failed to read blob {hash}: {err}")))?;

        Ok(Some(buf))
    }

    fn exists(&self, hash: &BlobHash) -> Result<bool> {
        Ok(self.blob_path(hash).exists())
    }

    fn put(&self, bytes: &[u8]) -> Result<BlobHash> {
        let hash = Self::digest(bytes);
        let path = self.blob_path(&hash);

        // Identical content already present at full length: refresh the mtime so a
        // concurrent purge sweep sees the blob as live, then skip the write.
        if fs::metadata(&path)
            .map(|m| m.len() as usize == bytes.len())
            .unwrap_or(false)
        {
            let touched = File::options()
                .append(true)
                .open(&path)
                .and_then(|file| file.set_modified(std::time::SystemTime::now()));
            if touched.is_ok() {
                return Ok(hash);
            }
        }

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                Error::store(format!(
                    "failed to create blob shard {}: {err}",
                    parent.display()
                ))
            })?;
        }

        // Stage in a sibling temp file and rename into place so a reader never sees a
        // half-written blob; the pid+seq name keeps concurrent puts from colliding.
        let stage = STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temp = path.with_extension(format!("tmp.{}.{stage}", std::process::id()));
        let mut file = File::create(&temp)
            .map_err(|err| Error::store(format!("failed to create blob {hash}: {err}")))?;
        file.write_all(bytes)
            .map_err(|err| Error::store(format!("failed to write blob {hash}: {err}")))?;
        file.sync_all()
            .map_err(|err| Error::store(format!("failed to flush blob {hash}: {err}")))?;
        drop(file);

        fs::rename(&temp, &path).map_err(|err| {
            let _ = fs::remove_file(&temp);
            Error::store(format!("failed to publish blob {hash}: {err}"))
        })?;

        Ok(hash)
    }

    fn delete(&self, hash: &BlobHash) -> Result<()> {
        let path = self.blob_path(hash);
        match fs::remove_file(&path) {
            Ok(()) => {
                self.prune_empty_shards(&path);
                Ok(())
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(Error::store(format!("failed to delete blob {hash}: {err}"))),
        }
    }

    fn modified_at(&self, hash: &BlobHash) -> Result<Option<u64>> {
        match fs::metadata(self.blob_path(hash)) {
            Ok(meta) => Ok(Some(unix_seconds(&meta))),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(Error::store(format!("failed to stat blob {hash}: {err}"))),
        }
    }

    fn for_each(&self, visit: &mut dyn FnMut(&BlobHash, u64) -> Result<()>) -> Result<()> {
        walk_blobs(&self.root, 0, visit)
    }

    fn usage_bytes(&self) -> Result<u64> {
        let mut total = 0u64;
        self.for_each(&mut |hash, _modified| {
            if let Ok(meta) = fs::metadata(self.blob_path(hash)) {
                total += meta.len();
            }
            Ok(())
        })?;
        Ok(total)
    }
}

fn walk_blobs(
    dir: &Path,
    depth: usize,
    visit: &mut dyn FnMut(&BlobHash, u64) -> Result<()>,
) -> Result<()> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => {
            return Err(Error::store(format!(
                "failed to list blob directory {}: {err}",
                dir.display()
            )))
        }
    };
    for entry in entries {
        let entry = entry.map_err(|err| {
            Error::store(format!(
                "failed to read blob directory {}: {err}",
                dir.display()
            ))
        })?;
        let path = entry.path();
        if path.is_dir() {
            if depth < FANOUT_LEVELS {
                walk_blobs(&path, depth + 1, visit)?;
            }
            continue;
        }
        let Some(hash) = hash_from_file_name(&path) else {
            continue;
        };
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        visit(&hash, unix_seconds(&meta))?;
    }
    Ok(())
}

fn hash_from_file_name(path: &Path) -> Option<BlobHash> {
    let name = path.file_name()?.to_str()?;
    if name.is_empty() || name.len() % 2 != 0 {
        return None;
    }
    let mut bytes = Vec::with_capacity(name.len() / 2);
    let digits = name.as_bytes();
    for pair in digits.chunks(2) {
        let high = (pair[0] as char).to_digit(16)?;
        let low = (pair[1] as char).to_digit(16)?;
        bytes.push(((high << 4) | low) as u8);
    }
    Some(BlobHash::from_bytes(bytes))
}

fn unix_seconds(meta: &fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "irixmail-blob-test-{}-{unique}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self { path }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn open_creates_the_root_directory_when_absent() {
        let dir = TempDir::new();
        let nested = dir.path.join("data").join("blobs");
        assert!(!nested.exists());

        let _store = FsBlobStore::open(&nested).unwrap();
        assert!(nested.exists());
    }

    #[test]
    fn put_returns_a_content_derived_hash() {
        let dir = TempDir::new();
        let store = FsBlobStore::open(&dir.path).unwrap();

        let one = store.put(b"hello world").unwrap();
        let two = store.put(b"hello world").unwrap();
        let other = store.put(b"goodbye").unwrap();

        assert_eq!(one, two);
        assert_ne!(one, other);
        assert_eq!(one.len(), 32);
    }

    #[test]
    fn put_then_get_all_round_trips_the_payload() {
        let dir = TempDir::new();
        let store = FsBlobStore::open(&dir.path).unwrap();

        let payload = b"the quick brown fox jumps over the lazy dog";
        let hash = store.put(payload).unwrap();

        assert_eq!(store.get_all(&hash).unwrap().as_deref(), Some(&payload[..]));
    }

    #[test]
    fn put_is_idempotent_for_identical_content() {
        let dir = TempDir::new();
        let store = FsBlobStore::open(&dir.path).unwrap();

        let hash = store.put(b"once").unwrap();
        let again = store.put(b"once").unwrap();
        assert_eq!(hash, again);
        assert_eq!(store.get_all(&hash).unwrap().as_deref(), Some(&b"once"[..]));
    }

    #[test]
    fn get_reads_a_byte_window() {
        let dir = TempDir::new();
        let store = FsBlobStore::open(&dir.path).unwrap();
        let hash = store.put(b"abcdefghij").unwrap();

        assert_eq!(
            store.get(&hash, 2..5).unwrap().as_deref(),
            Some(&b"cde"[..])
        );
    }

    #[test]
    fn get_clamps_a_range_past_the_end() {
        let dir = TempDir::new();
        let store = FsBlobStore::open(&dir.path).unwrap();
        let hash = store.put(b"abc").unwrap();

        assert_eq!(
            store.get(&hash, 1..99).unwrap().as_deref(),
            Some(&b"bc"[..])
        );
        assert_eq!(
            store.get(&hash, 0..usize::MAX).unwrap().as_deref(),
            Some(&b"abc"[..])
        );
    }

    #[test]
    fn get_an_empty_or_out_of_range_window_is_empty() {
        let dir = TempDir::new();
        let store = FsBlobStore::open(&dir.path).unwrap();
        let hash = store.put(b"abc").unwrap();

        assert_eq!(store.get(&hash, 1..1).unwrap().as_deref(), Some(&b""[..]));
        assert_eq!(store.get(&hash, 9..12).unwrap().as_deref(), Some(&b""[..]));
    }

    #[test]
    fn get_missing_blob_is_none() {
        let dir = TempDir::new();
        let store = FsBlobStore::open(&dir.path).unwrap();
        let absent = BlobHash::from_bytes(vec![0u8; 32]);

        assert_eq!(store.get_all(&absent).unwrap(), None);
        assert!(!store.exists(&absent).unwrap());
    }

    #[test]
    fn exists_tracks_presence() {
        let dir = TempDir::new();
        let store = FsBlobStore::open(&dir.path).unwrap();
        let hash = store.put(b"present").unwrap();

        assert!(store.exists(&hash).unwrap());
    }

    #[test]
    fn delete_removes_then_is_a_no_op() {
        let dir = TempDir::new();
        let store = FsBlobStore::open(&dir.path).unwrap();
        let hash = store.put(b"transient").unwrap();
        assert!(store.exists(&hash).unwrap());

        store.delete(&hash).unwrap();
        assert!(!store.exists(&hash).unwrap());
        store.delete(&hash).unwrap();
    }

    #[test]
    fn delete_prunes_empty_shard_directories() {
        let dir = TempDir::new();
        let store = FsBlobStore::open(&dir.path).unwrap();
        let hash = store.put(b"sharded").unwrap();

        let blob_path = store.blob_path(&hash);
        let top_shard = blob_path
            .parent()
            .and_then(Path::parent)
            .unwrap()
            .to_path_buf();
        assert!(top_shard.exists());

        store.delete(&hash).unwrap();

        assert!(!top_shard.exists());
        assert!(dir.path.exists());
    }

    #[test]
    fn delete_keeps_shards_shared_by_another_blob() {
        let dir = TempDir::new();
        let store = FsBlobStore::open(&dir.path).unwrap();

        let a = store.put(b"alpha").unwrap();
        let b = store.put(b"beta").unwrap();

        store.delete(&a).unwrap();
        assert_eq!(store.get_all(&b).unwrap().as_deref(), Some(&b"beta"[..]));
        store.delete(&b).unwrap();
        assert!(dir.path.exists());
    }

    #[test]
    fn blobs_are_addressable_after_reopening() {
        let dir = TempDir::new();
        let hash = {
            let store = FsBlobStore::open(&dir.path).unwrap();
            store.put(b"durable bytes").unwrap()
        };
        let store = FsBlobStore::open(&dir.path).unwrap();
        assert_eq!(
            store.get_all(&hash).unwrap().as_deref(),
            Some(&b"durable bytes"[..])
        );
    }

    #[test]
    fn blob_path_fans_out_by_digest_prefix() {
        let dir = TempDir::new();
        let store = FsBlobStore::open(&dir.path).unwrap();
        let hash = store.put(b"layout").unwrap();
        let hex = hash.to_hex();

        let path = store.blob_path(&hash);
        assert_eq!(path.file_name().unwrap().to_str().unwrap(), hex);
        let level_two = path.parent().unwrap();
        let level_one = level_two.parent().unwrap();
        assert_eq!(level_two.file_name().unwrap().to_str().unwrap(), &hex[2..4]);
        assert_eq!(level_one.file_name().unwrap().to_str().unwrap(), &hex[0..2]);
        assert_eq!(level_one.parent().unwrap(), dir.path);
    }

    #[test]
    fn concurrent_puts_of_identical_content_both_succeed() {
        use std::sync::Arc;

        let dir = TempDir::new();
        let store = Arc::new(FsBlobStore::open(&dir.path).unwrap());
        let payload = b"the same bytes staged from two threads at once";

        let handles: Vec<_> = (0..2)
            .map(|_| {
                let store = Arc::clone(&store);
                std::thread::spawn(move || store.put(payload).unwrap())
            })
            .collect();
        let hashes: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();

        assert_eq!(hashes[0], hashes[1]);
        assert_eq!(
            store.get_all(&hashes[0]).unwrap().as_deref(),
            Some(&payload[..])
        );
    }

    #[test]
    fn fs_blob_store_is_a_shareable_blob_store() {
        let dir = TempDir::new();
        let _boxed: std::sync::Arc<dyn BlobStore> =
            std::sync::Arc::new(FsBlobStore::open(&dir.path).unwrap());
    }

    #[test]
    fn usage_bytes_sums_the_stored_blob_sizes() {
        let dir = TempDir::new();
        let store = FsBlobStore::open(&dir.path).unwrap();
        store.put(b"ten bytes.").unwrap();
        store.put(b"five!").unwrap();

        assert_eq!(store.usage_bytes().unwrap(), 15);
    }

    #[test]
    fn for_each_enumerates_every_stored_blob_with_its_mtime() {
        let dir = TempDir::new();
        let store = FsBlobStore::open(&dir.path).unwrap();
        let one = store.put(b"first blob").unwrap();
        let two = store.put(b"second blob").unwrap();

        let mut seen = Vec::new();
        store
            .for_each(&mut |hash, modified| {
                assert!(modified > 0);
                seen.push(hash.clone());
                Ok(())
            })
            .unwrap();
        seen.sort();
        let mut expected = vec![one, two];
        expected.sort();
        assert_eq!(seen, expected);
    }

    #[test]
    fn a_dedup_put_refreshes_the_blob_mtime() {
        let dir = TempDir::new();
        let store = FsBlobStore::open(&dir.path).unwrap();
        let hash = store.put(b"long lived content").unwrap();

        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(30 * 24 * 3600);
        File::options()
            .append(true)
            .open(store.blob_path(&hash))
            .unwrap()
            .set_modified(old)
            .unwrap();
        let aged = store.modified_at(&hash).unwrap().unwrap();

        store.put(b"long lived content").unwrap();
        let refreshed = store.modified_at(&hash).unwrap().unwrap();
        assert!(
            refreshed > aged + 24 * 3600,
            "a dedup put must refresh the mtime so the purge grace window protects it"
        );
    }
}
