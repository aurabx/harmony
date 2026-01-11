//! Default query provider implementation

use async_trait::async_trait;
use std::path::PathBuf;
use tracing::{info, warn};

use crate::types::{DatasetStream, QueryLevel};
use crate::Result;

use super::QueryProvider;

/// Default query provider implementation (for testing)
pub struct DefaultQueryProvider {
    storage_dir: PathBuf,
}

impl DefaultQueryProvider {
    pub fn new(storage_dir: PathBuf) -> Self {
        Self { storage_dir }
    }
}

#[async_trait]
impl QueryProvider for DefaultQueryProvider {
    async fn find(
        &self,
        _query_level: QueryLevel,
        _parameters: &std::collections::HashMap<String, String>,
        _max_results: u32,
    ) -> Result<Vec<DatasetStream>> {
        // TODO: Implement actual query logic
        warn!("DefaultQueryProvider::find not yet implemented");
        Ok(vec![])
    }

    async fn locate(
        &self,
        _query_level: QueryLevel,
        _parameters: &std::collections::HashMap<String, String>,
    ) -> Result<Vec<DatasetStream>> {
        // TODO: Implement actual locate logic
        warn!("DefaultQueryProvider::locate not yet implemented");
        Ok(vec![])
    }

    async fn get(
        &self,
        _query_level: QueryLevel,
        _parameters: &std::collections::HashMap<String, String>,
    ) -> Result<Vec<DatasetStream>> {
        // TODO: Implement actual get logic
        warn!("DefaultQueryProvider::get not yet implemented");
        Ok(vec![])
    }

    async fn store(&self, dataset: DatasetStream) -> Result<()> {
        // Store the dataset to the storage directory
        let temp_file = dataset.to_temp_file(&self.storage_dir).await?;
        info!("Stored dataset to {}", temp_file.display());
        Ok(())
    }
}
