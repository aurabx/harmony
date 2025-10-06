//! Internal router for decoupling DIMSE operations from HTTP layer

use async_trait::async_trait;
use futures::stream::BoxStream;
// use serde::{Deserialize, Serialize}; // TODO: Used when implementing actual message serialization
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use crate::types::{DatasetStream, DimseCommand, FindQuery, MoveQuery};
use crate::{DimseError, RemoteNode, Result};

/// Request sent to the DIMSE router
#[derive(Debug)]
pub struct DimseRequest {
    /// Unique request ID for correlation
    pub id: Uuid,
    
    /// The DIMSE command to execute
    pub command: DimseCommand,
    
    /// Target remote node (for SCU operations)
    pub remote_node: Option<RemoteNode>,
    
    /// Request payload
    pub payload: DimseRequestPayload,
    
    /// Response channel (for single responses)
    pub response_tx: Option<oneshot::Sender<DimseResponse>>,
    
    /// Stream response channel (for streaming responses like C-FIND)
    pub stream_tx: Option<mpsc::Sender<DimseResponse>>,
}

/// Response from the DIMSE router
#[derive(Debug, Clone)]
pub struct DimseResponse {
    /// Request ID this response correlates to
    pub request_id: Uuid,
    
    /// The response payload
    pub payload: DimseResponsePayload,
    
    /// Whether this is the final response in a sequence
    pub is_final: bool,
}

/// Payload types for DIMSE requests
#[derive(Debug, Clone)]
pub enum DimseRequestPayload {
    /// C-ECHO request (no additional data needed)
    Echo,
    
    /// C-FIND request with query parameters
    Find(FindQuery),
    
    /// C-MOVE request with query and destination
    Move(MoveQuery),
    
    /// C-STORE request with dataset to store
    Store(DatasetStream),
}

/// Payload types for DIMSE responses
#[derive(Debug, Clone)]
pub enum DimseResponsePayload {
    /// C-ECHO response (success/failure)
    Echo { success: bool },
    
    /// C-FIND response with matching dataset
    Find { dataset: Option<DatasetStream> },
    
    /// C-MOVE response with moved dataset or status update
    Move { 
        dataset: Option<DatasetStream>,
        remaining: u32,
        completed: u32,
        failed: u32,
        warning: u32,
    },
    
    /// C-STORE response (success/failure)
    Store { success: bool },
    
    /// Error response
    Error { error: String },
}

/// Router trait for handling DIMSE operations
#[async_trait]
pub trait Router: Send + Sync {
    /// Send a request and wait for a single response
    async fn send_request(&self, request: DimseRequest) -> Result<DimseResponse>;
    
    /// Send a request and get a stream of responses (for C-FIND, C-MOVE)
    async fn send_streaming_request(&self, request: DimseRequest) -> Result<BoxStream<'static, DimseResponse>>;
    
    /// Get the next available request (for SCP implementations)
    async fn next_request(&mut self) -> Result<DimseRequest>;
    
    /// Send a response (for SCP implementations)
    async fn send_response(&self, response: DimseResponse) -> Result<()>;
}

/// In-memory router implementation using tokio channels
pub struct InMemoryRouter {
    /// Channel for sending requests from HTTP layer to DIMSE layer
    request_tx: mpsc::Sender<DimseRequest>,
    /// Channel for receiving requests in DIMSE layer
    request_rx: Option<mpsc::Receiver<DimseRequest>>,
    /// Channel for sending responses from DIMSE layer to HTTP layer
    response_tx: mpsc::Sender<DimseResponse>,
    /// Channel for receiving responses in HTTP layer
    response_rx: Option<mpsc::Receiver<DimseResponse>>,
}

impl InMemoryRouter {
    /// Create a new in-memory router with default buffer sizes
    pub fn new() -> Self {
        Self::with_buffer_size(1000)
    }
    
    /// Create a new in-memory router with specified buffer size
    pub fn with_buffer_size(buffer_size: usize) -> Self {
        let (request_tx, request_rx) = mpsc::channel(buffer_size);
        let (response_tx, response_rx) = mpsc::channel(buffer_size);
        
        Self {
            request_tx,
            request_rx: Some(request_rx),
            response_tx,
            response_rx: Some(response_rx),
        }
    }
    
    /// Split the router into sender and receiver halves
    pub fn split(mut self) -> (RouterSender, RouterReceiver) {
        let sender = RouterSender {
            request_tx: self.request_tx.clone(),
            response_rx: self.response_rx.take().expect("Router already split"),
        };
        
        let receiver = RouterReceiver {
            request_rx: self.request_rx.take().expect("Router already split"),
            response_tx: self.response_tx.clone(),
        };
        
        (sender, receiver)
    }
}

impl Default for InMemoryRouter {
    fn default() -> Self {
        Self::new()
    }
}

/// Sender half of the router (used by HTTP layer)
pub struct RouterSender {
    request_tx: mpsc::Sender<DimseRequest>,
    #[allow(dead_code)]
    response_rx: mpsc::Receiver<DimseResponse>, // TODO: Used for bidirectional communication
}

/// Receiver half of the router (used by DIMSE layer)
pub struct RouterReceiver {
    request_rx: mpsc::Receiver<DimseRequest>,
    response_tx: mpsc::Sender<DimseResponse>,
}

#[async_trait]
impl Router for RouterSender {
    async fn send_request(&self, mut request: DimseRequest) -> Result<DimseResponse> {
        let (response_tx, response_rx) = oneshot::channel();
        request.response_tx = Some(response_tx);
        
        self.request_tx.send(request)
            .await
            .map_err(|_| DimseError::router("Failed to send request"))?;
        
        response_rx.await
            .map_err(|_| DimseError::router("Failed to receive response"))
    }
    
    async fn send_streaming_request(&self, mut request: DimseRequest) -> Result<BoxStream<'static, DimseResponse>> {
        let (stream_tx, mut stream_rx) = mpsc::channel(100);
        request.stream_tx = Some(stream_tx);
        
        self.request_tx.send(request)
            .await
            .map_err(|_| DimseError::router("Failed to send streaming request"))?;
        
        let stream = async_stream::stream! {
            while let Some(response) = stream_rx.recv().await {
                let is_final = response.is_final;
                yield response;
                if is_final {
                    break;
                }
            }
        };
        
        Ok(Box::pin(stream))
    }
    
    async fn next_request(&mut self) -> Result<DimseRequest> {
        Err(DimseError::operation_failed("RouterSender cannot receive requests"))
    }
    
    async fn send_response(&self, _response: DimseResponse) -> Result<()> {
        Err(DimseError::operation_failed("RouterSender cannot send responses"))
    }
}

#[async_trait]
impl Router for RouterReceiver {
    async fn send_request(&self, _request: DimseRequest) -> Result<DimseResponse> {
        Err(DimseError::operation_failed("RouterReceiver cannot send requests"))
    }
    
    async fn send_streaming_request(&self, _request: DimseRequest) -> Result<BoxStream<'static, DimseResponse>> {
        Err(DimseError::operation_failed("RouterReceiver cannot send requests"))
    }
    
    async fn next_request(&mut self) -> Result<DimseRequest> {
        self.request_rx.recv()
            .await
            .ok_or_else(|| DimseError::router("Request channel closed"))
    }
    
    async fn send_response(&self, response: DimseResponse) -> Result<()> {
        self.response_tx.send(response)
            .await
            .map_err(|_| DimseError::router("Failed to send response"))
    }
}

impl DimseRequest {
    /// Create a new C-ECHO request
    pub fn echo(remote_node: RemoteNode) -> Self {
        Self {
            id: Uuid::new_v4(),
            command: DimseCommand::Echo,
            remote_node: Some(remote_node),
            payload: DimseRequestPayload::Echo,
            response_tx: None,
            stream_tx: None,
        }
    }
    
    /// Create a new C-FIND request
    pub fn find(remote_node: RemoteNode, query: FindQuery) -> Self {
        Self {
            id: Uuid::new_v4(),
            command: DimseCommand::Find,
            remote_node: Some(remote_node),
            payload: DimseRequestPayload::Find(query),
            response_tx: None,
            stream_tx: None,
        }
    }
    
    /// Create a new C-MOVE request
    pub fn move_request(remote_node: RemoteNode, query: MoveQuery) -> Self {
        Self {
            id: Uuid::new_v4(),
            command: DimseCommand::Move,
            remote_node: Some(remote_node),
            payload: DimseRequestPayload::Move(query),
            response_tx: None,
            stream_tx: None,
        }
    }
    
    /// Create a new C-STORE request
    pub fn store(remote_node: RemoteNode, dataset: DatasetStream) -> Self {
        Self {
            id: Uuid::new_v4(),
            command: DimseCommand::Store,
            remote_node: Some(remote_node),
            payload: DimseRequestPayload::Store(dataset),
            response_tx: None,
            stream_tx: None,
        }
    }
}

impl DimseResponse {
    /// Create a new C-ECHO response
    pub fn echo(request_id: Uuid, success: bool) -> Self {
        Self {
            request_id,
            payload: DimseResponsePayload::Echo { success },
            is_final: true,
        }
    }
    
    /// Create a new C-FIND response
    pub fn find(request_id: Uuid, dataset: Option<DatasetStream>, is_final: bool) -> Self {
        Self {
            request_id,
            payload: DimseResponsePayload::Find { dataset },
            is_final,
        }
    }
    
    /// Create a new C-MOVE response
    pub fn move_response(
        request_id: Uuid,
        dataset: Option<DatasetStream>,
        remaining: u32,
        completed: u32,
        failed: u32,
        warning: u32,
        is_final: bool,
    ) -> Self {
        Self {
            request_id,
            payload: DimseResponsePayload::Move {
                dataset,
                remaining,
                completed,
                failed,
                warning,
            },
            is_final,
        }
    }
    
    /// Create a new C-STORE response
    pub fn store(request_id: Uuid, success: bool) -> Self {
        Self {
            request_id,
            payload: DimseResponsePayload::Store { success },
            is_final: true,
        }
    }
    
    /// Create a new error response
    pub fn error(request_id: Uuid, error: String) -> Self {
        Self {
            request_id,
            payload: DimseResponsePayload::Error { error },
            is_final: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::QueryLevel;

    #[tokio::test]
    async fn test_router_echo() {
        let router = InMemoryRouter::new();
        let (sender, receiver) = router.split();
        
        // Spawn a task to handle the request
        let handle = tokio::spawn(async move {
            let mut receiver = receiver;
            let request = receiver.next_request().await.unwrap();
            let response = DimseResponse::echo(request.id, true);
            
            if let Some(tx) = request.response_tx {
                tx.send(response).unwrap();
            }
        });
        
        // Send an echo request
        let remote_node = RemoteNode::new("TEST_AET", "localhost", 11112);
        let request = DimseRequest::echo(remote_node);
        let response = sender.send_request(request).await.unwrap();
        
        match response.payload {
            DimseResponsePayload::Echo { success } => assert!(success),
            _ => panic!("Expected echo response"),
        }
        
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn test_request_builders() {
        let remote_node = RemoteNode::new("TEST_AET", "localhost", 11112);
        
        // Test echo request
        let echo_req = DimseRequest::echo(remote_node.clone());
        assert_eq!(echo_req.command, DimseCommand::Echo);
        
        // Test find request
        let query = FindQuery::patient(Some("12345".to_string()));
        let find_req = DimseRequest::find(remote_node.clone(), query);
        assert_eq!(find_req.command, DimseCommand::Find);
        
        // Test move request
        let move_query = MoveQuery::new(QueryLevel::Patient, "DEST_AET");
        let move_req = DimseRequest::move_request(remote_node, move_query);
        assert_eq!(move_req.command, DimseCommand::Move);
    }
}