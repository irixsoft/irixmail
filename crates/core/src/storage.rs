use std::any::Any;
use std::sync::Arc;

use crate::error::{Error, Result};

type ErasedHandle = Arc<dyn Any + Send + Sync>;

#[derive(Clone)]
pub struct Storage {
    store: ErasedHandle,
    blob_store: ErasedHandle,
}

impl Storage {
    pub fn new<S, B>(store: Arc<S>, blob_store: Arc<B>) -> Self
    where
        S: Any + Send + Sync,
        B: Any + Send + Sync,
    {
        Self { store, blob_store }
    }

    pub fn store<T>(&self) -> Result<Arc<T>>
    where
        T: Any + Send + Sync,
    {
        downcast(&self.store, "key-value store")
    }

    pub fn blob_store<T>(&self) -> Result<Arc<T>>
    where
        T: Any + Send + Sync,
    {
        downcast(&self.blob_store, "blob store")
    }
}

fn downcast<T>(handle: &ErasedHandle, label: &str) -> Result<Arc<T>>
where
    T: Any + Send + Sync,
{
    handle.clone().downcast::<T>().map_err(|_| {
        Error::store(format!(
            "{label} was not opened as the requested backend type"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeStore {
        name: &'static str,
    }

    struct FakeBlobStore {
        root: &'static str,
    }

    fn storage() -> Storage {
        Storage::new(
            Arc::new(FakeStore { name: "kv" }),
            Arc::new(FakeBlobStore { root: "/blobs" }),
        )
    }

    #[test]
    fn each_backend_is_recovered_as_its_concrete_type() {
        let storage = storage();

        let store = storage.store::<FakeStore>().expect("store recovers");
        assert_eq!(store.name, "kv");

        let blobs = storage
            .blob_store::<FakeBlobStore>()
            .expect("blob store recovers");
        assert_eq!(blobs.root, "/blobs");
    }

    #[test]
    fn recovering_the_wrong_type_is_a_storage_error() {
        let storage = storage();

        let wrong = storage.store::<FakeBlobStore>();
        assert!(matches!(wrong, Err(Error::Store(_))));

        let wrong = storage.blob_store::<FakeStore>();
        assert!(matches!(wrong, Err(Error::Store(_))));
    }

    #[test]
    fn recovered_handles_share_the_same_allocation() {
        let storage = storage();

        let first = storage.store::<FakeStore>().expect("first recovers");
        let second = storage.store::<FakeStore>().expect("second recovers");
        assert!(Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn cloning_the_bundle_shares_the_open_handles() {
        let storage = storage();
        let cloned = storage.clone();

        let original = storage.store::<FakeStore>().expect("original recovers");
        let from_clone = cloned.store::<FakeStore>().expect("clone recovers");
        assert!(Arc::ptr_eq(&original, &from_clone));
    }
}
