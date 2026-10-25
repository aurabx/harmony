//! Service Class User (SCU) implementation for outbound DIMSE operations

pub mod commands;
pub mod connection;
pub mod native_connection;
pub mod storage_sop_classes;
pub mod command_builder;

use tokio_stream::wrappers::ReceiverStream;

use crate::config::{DimseConfig, RemoteNode};
use crate::scu::commands::{echo, find, get, r#move, store};
use crate::types::{DatasetStream, FindQuery, GetQuery, MoveQuery};
use crate::Result;

/// DIMSE Service Class User
pub struct DimseScu {
    config: DimseConfig,
}

impl DimseScu {
    /// Create a new SCU with the given configuration
    pub fn new(config: DimseConfig) -> Self {
        Self { config }
    }

    /// Send a C-ECHO request to a remote node
    pub async fn echo(&self, node: &RemoteNode) -> Result<bool> {
        echo::handle_echo(&self.config, node).await
    }

    /// Send a C-FIND request to a remote node
    pub async fn find(
        &self,
        node: &RemoteNode,
        query: FindQuery,
    ) -> Result<ReceiverStream<Result<DatasetStream>>> {
        find::handle_find(&self.config, node, query).await
    }

    /// Send a C-MOVE request to a remote node
    pub async fn move_request(
        &self,
        node: &RemoteNode,
        query: MoveQuery,
        output_dir: Option<std::path::PathBuf>,
    ) -> Result<ReceiverStream<Result<DatasetStream>>> {
        r#move::handle_move(
            &self.config,
            node,
            query,
            output_dir,
            self.config.external_store_scp,
            self.config.incoming_store_port,
        )
        .await
    }

    /// Send a C-GET request to a remote node
    pub async fn get_request(
        &self,
        node: &RemoteNode,
        query: GetQuery,
        output_dir: Option<std::path::PathBuf>,
    ) -> Result<ReceiverStream<Result<DatasetStream>>> {
        get::handle_get(&self.config, node, query, output_dir).await
    }

    /// Send a C-STORE request to a remote node
    pub async fn store(&self, node: &RemoteNode, dataset: DatasetStream) -> Result<bool> {
        store::handle_store(&self.config, node, dataset).await
    }

    /// Test connectivity to a remote node with retry logic
    pub async fn test_connection(&self, node: &RemoteNode, max_retries: u32) -> Result<bool> {
        connection::test_connection(&self.config, node, max_retries).await
    }

    /// Get connection timeout for a node (uses node-specific or global setting)
    #[allow(dead_code)]
    pub fn get_connection_timeout(&self, node: &RemoteNode) -> std::time::Duration {
        connection::get_connection_timeout(&self.config, node)
    }

    /// Get maximum PDU size for a node (uses node-specific or global setting)
    #[allow(dead_code)]
    pub fn get_max_pdu(&self, node: &RemoteNode) -> u32 {
        connection::get_max_pdu(&self.config, node)
    }
}

/// Builder for creating SCU instances with custom configurations
pub struct ScuBuilder {
    config: DimseConfig,
}

impl ScuBuilder {
    /// Start building a new SCU
    pub fn new() -> Self {
        Self {
            config: DimseConfig::default(),
        }
    }

    /// Set the local AE title
    pub fn local_aet(mut self, aet: impl Into<String>) -> Self {
        self.config.local_aet = aet.into();
        self
    }

    /// Set the connection timeout
    pub fn connection_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.config.connect_timeout_ms = timeout.as_millis() as u64;
        self
    }

    /// Set the maximum PDU size
    pub fn max_pdu(mut self, size: u32) -> Self {
        self.config.max_pdu = size;
        self
    }

    /// Build the SCU
    pub fn build(self) -> Result<DimseScu> {
        self.config.validate()?;
        Ok(DimseScu::new(self.config))
    }
}

impl Default for ScuBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::stream::StreamExt;
    use std::time::Duration;

    #[tokio::test]
    async fn test_scu_creation() {
        let config = DimseConfig {
            local_aet: "TEST_SCU".to_string(),
            connect_timeout_ms: 10_000,
            ..Default::default()
        };
        let scu = DimseScu::new(config);
        
        // Test that SCU was created successfully
        let node = RemoteNode::new("TEST_AET", "localhost", 11112);
        assert_eq!(
            scu.get_connection_timeout(&node),
            Duration::from_millis(10_000)
        );
    }

    #[tokio::test]
    #[ignore]
    async fn test_echo_stub() {
        let scu = DimseScu::new(DimseConfig::default());
        let node = RemoteNode::new("TEST_AET", "localhost", 11112);

        // This should succeed with our stub implementation
        let result = scu.echo(&node).await;
        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    /// A local port with nothing listening on it.
    fn closed_port() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind a free port");
        let port = listener.local_addr().expect("local address").port();
        drop(listener);
        port
    }

    /// An unreachable node must show up as an error on the stream. An empty
    /// stream would read as "no matches".
    #[tokio::test]
    async fn test_find_unreachable_node_yields_error() {
        let scu = DimseScu::new(DimseConfig::default());
        let node = RemoteNode::new("TEST_AET", "127.0.0.1", closed_port());
        let query = FindQuery::patient(Some("12345".to_string()));

        let mut stream = scu.find(&node, query).await.unwrap();

        match stream.next().await {
            Some(Err(_)) => {}
            Some(Ok(_)) => panic!("expected an error, got a dataset"),
            None => panic!("expected an error, got an empty stream"),
        }
        assert!(
            stream.next().await.is_none(),
            "the error must end the stream"
        );
    }

    #[tokio::test]
    async fn test_get_unreachable_node_yields_error() {
        let scu = DimseScu::new(DimseConfig::default());
        let node = RemoteNode::new("TEST_AET", "127.0.0.1", closed_port());
        let query = GetQuery::new(crate::types::QueryLevel::Study)
            .with_parameter("0020000D".to_string(), "1.2.3".to_string());

        let mut stream = scu.get_request(&node, query, None).await.unwrap();

        assert!(
            matches!(stream.next().await, Some(Err(_))),
            "expected an error on the stream"
        );
    }

    /// Covers both a missing movescu binary and a movescu run that cannot
    /// connect. Either way the caller must see an error.
    #[tokio::test]
    async fn test_move_unreachable_node_yields_error() {
        let storage_dir = std::env::temp_dir().join(format!("dimse-move-{}", uuid::Uuid::new_v4()));
        let scu = DimseScu::new(DimseConfig {
            storage_dir: storage_dir.clone(),
            incoming_store_port: closed_port(),
            ..Default::default()
        });
        let node = RemoteNode::new("TEST_AET", "127.0.0.1", closed_port());
        let query = MoveQuery::new(crate::types::QueryLevel::Study, "TEST_DEST")
            .with_parameter("0020000D".to_string(), "1.2.3".to_string());

        let mut stream = scu.move_request(&node, query, None).await.unwrap();
        let first = stream.next().await;
        let _ = std::fs::remove_dir_all(&storage_dir);

        assert!(
            matches!(first, Some(Err(_))),
            "expected an error on the stream"
        );
    }

    #[tokio::test]
    async fn test_connection_timeout_selection() {
        let scu = DimseScu::new(DimseConfig {
            connect_timeout_ms: 5000,
            ..Default::default()
        });

        // Node without specific timeout should use global
        let node1 = RemoteNode::new("TEST1", "localhost", 11112);
        assert_eq!(
            scu.get_connection_timeout(&node1),
            Duration::from_millis(5000)
        );

        // Node with specific timeout should use its own
        let node2 = RemoteNode::new("TEST2", "localhost", 11113).with_timeout(2000);
        assert_eq!(
            scu.get_connection_timeout(&node2),
            Duration::from_millis(2000)
        );
    }

    #[test]
    fn test_invalid_config_validation() {
        let result = ScuBuilder::new()
            .local_aet("") // Invalid empty AE title
            .build();

        assert!(result.is_err());
    }
}
