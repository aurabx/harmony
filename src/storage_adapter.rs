/// Adapter to bridge harmony-filesystem StorageBackend to runbeam-sdk StorageBackend
///
/// This adapter allows harmony-proxy's FilesystemStorage to be used with runbeam-sdk
/// functions that require the SDK's StorageBackend trait.
use harmony_filesystem::StorageBackend as HarmonyStorage;
use runbeam_sdk::storage::StorageBackend as SdkStorage;
use runbeam_sdk::storage::StorageError as SdkStorageError;
use std::future::Future;
use std::pin::Pin;

/// Wrapper that adapts harmony-filesystem's StorageBackend to runbeam-sdk's StorageBackend
pub struct StorageAdapter<'a> {
    inner: &'a dyn HarmonyStorage,
}

impl<'a> StorageAdapter<'a> {
    pub fn new(inner: &'a dyn HarmonyStorage) -> Self {
        Self { inner }
    }
}

impl<'a> SdkStorage for StorageAdapter<'a> {
    fn write_file_str(
        &self,
        path: &str,
        data: &[u8],
    ) -> Pin<Box<dyn Future<Output = Result<(), SdkStorageError>> + Send + '_>> {
        let path = path.to_string();
        let data = data.to_vec();

        Box::pin(async move {
            self.inner
                .write_file_str(&path, &data)
                .await
                .map(|_| ()) // Discard the PathBuf return value
                .map_err(|e| match e {
                    harmony_filesystem::StorageError::Io(io_err) => {
                        SdkStorageError::Io(io_err)
                    }
                    harmony_filesystem::StorageError::Path(msg) => SdkStorageError::Path(msg),
                    harmony_filesystem::StorageError::Config(msg) => {
                        SdkStorageError::Config(msg)
                    }
                })
        })
    }

    fn read_file_str(
        &self,
        path: &str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, SdkStorageError>> + Send + '_>> {
        let path = path.to_string();

        Box::pin(async move {
            self.inner
                .read_file_str(&path)
                .await
                .map_err(|e| match e {
                    harmony_filesystem::StorageError::Io(io_err) => {
                        SdkStorageError::Io(io_err)
                    }
                    harmony_filesystem::StorageError::Path(msg) => SdkStorageError::Path(msg),
                    harmony_filesystem::StorageError::Config(msg) => {
                        SdkStorageError::Config(msg)
                    }
                })
        })
    }

    fn exists_str(&self, path: &str) -> bool {
        self.inner.exists_str(path)
    }

    fn remove_str(
        &self,
        path: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), SdkStorageError>> + Send + '_>> {
        let path = path.to_string();

        Box::pin(async move {
            self.inner.remove_str(&path).await.map_err(|e| match e {
                harmony_filesystem::StorageError::Io(io_err) => SdkStorageError::Io(io_err),
                harmony_filesystem::StorageError::Path(msg) => SdkStorageError::Path(msg),
                harmony_filesystem::StorageError::Config(msg) => SdkStorageError::Config(msg),
            })
        })
    }
}
