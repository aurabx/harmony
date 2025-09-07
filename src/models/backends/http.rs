use crate::models::envelope::envelope::Envelope;
use async_trait::async_trait;

#[async_trait]
pub trait Backend: Send + Sync {
    async fn process(&self, envelope: Envelope) -> Result<Envelope, crate::models::middleware::types::Error>;
}

pub struct HttpBackend;

#[async_trait]
impl Backend for HttpBackend {
    async fn process(&self, envelope: Envelope) -> Result<Envelope, crate::models::middleware::types::Error> {
        // Convert Envelope into an outgoing HTTP request using details
        // Call external system, retrieve response

        // Convert response into Envelope
        Ok(Envelope {
            request_details: envelope.request_details.clone(),
            original_data: "response".to_string(), // Example response data
            normalized_data: None,
        })
    }
}
