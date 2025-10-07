use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Protocol {
    Http,
    Dimse,
    Hl7V2Mllp,
    Sftp,
    Scp,
    Amqp,
    Mqtt,
    Nats,
    Kafka,
    WebRtc,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProtocolCtx {
    pub protocol: Protocol,
    // Primary payload for the event/message/request
    pub payload: Vec<u8>,
    // Simple key/value metadata (e.g., calling_aet, routing_key)
    pub meta: HashMap<String, String>,
    // Rich structured attributes (e.g., headers map, cookies, query params, protocol-specific fields)
    pub attrs: Value,
}
