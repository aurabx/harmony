//! Command object for expressing protocol-agnostic operations.
//!
//! The `Command` struct expresses the intended operation in universal terms.
//! Endpoints or transforms set the command, and backends interpret it according
//! to their protocol.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A protocol-agnostic command expressing the intended operation.
///
/// # Example
///
/// ```text
/// let cmd = Command::new(Operation::Query)
///     .with_source("my_transform");
/// ```
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Command {
    /// The operation to perform
    pub operation: Operation,

    /// Operation-specific parameters (flexible key-value)
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub params: HashMap<String, serde_json::Value>,

    /// Tracks where this command was set (for debugging)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

impl Command {
    /// Create a new command with the given operation.
    pub fn new(operation: Operation) -> Self {
        Self {
            operation,
            params: HashMap::new(),
            source: None,
        }
    }

    /// Create a command with operation and parameters.
    pub fn with_params(operation: Operation, params: HashMap<String, serde_json::Value>) -> Self {
        Self {
            operation,
            params,
            source: None,
        }
    }

    /// Set the source for debugging/tracing purposes.
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    /// Add a parameter to the command.
    pub fn with_param(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.params.insert(key.into(), value);
        self
    }
}

/// Universal operations that can be mapped to any protocol.
///
/// | Operation   | DICOM SCU    | DICOMweb   | HTTP        | FHIR     |
/// |-------------|--------------|------------|-------------|----------|
/// | `Query`     | C-FIND       | QIDO-RS    | GET         | search   |
/// | `Retrieve`  | C-GET        | WADO-RS    | GET         | read     |
/// | `Move`      | C-MOVE       | -          | -           | -        |
/// | `Store`     | C-STORE      | STOW-RS    | POST        | create   |
/// | `Update`    | -            | -          | PUT         | update   |
/// | `Patch`     | -            | -          | PATCH       | patch    |
/// | `Delete`    | -            | DELETE     | DELETE      | delete   |
/// | `Ping`      | C-ECHO       | -          | HEAD/OPTIONS| metadata |
/// | `Subscribe` | -            | -          | -           | Subscription |
/// | `Publish`   | -            | -          | -           | -        |
/// | `Custom`    | (action key) | (action key)| (action key)| (action key) |
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    /// Search/query for resources matching criteria.
    /// DICOM: C-FIND | DICOMweb: QIDO-RS | HTTP: GET | FHIR: search
    Query,

    /// Retrieve a specific resource or set of resources.
    /// DICOM: C-GET | DICOMweb: WADO-RS | HTTP: GET | FHIR: read
    Retrieve,

    /// Move/relocate a resource (push to a destination).
    /// DICOM: C-MOVE
    Move,

    /// Store/create a new resource.
    /// DICOM: C-STORE | DICOMweb: STOW-RS | HTTP: POST | FHIR: create
    Store,

    /// Update an existing resource (full replacement).
    /// HTTP: PUT | FHIR: update
    Update,

    /// Partially modify an existing resource.
    /// HTTP: PATCH | FHIR: patch
    Patch,

    /// Remove a resource.
    /// HTTP: DELETE | FHIR: delete
    Delete,

    /// Connectivity/health check.
    /// DICOM: C-ECHO | HTTP: HEAD/OPTIONS | FHIR: metadata
    Ping,

    /// Subscribe to events/changes.
    /// FHIR: Subscription | AMQP/MQTT/etc: subscribe
    Subscribe,

    /// Publish/push a message or event.
    /// AMQP/MQTT/etc: publish
    Publish,

    /// Execute a custom action.
    /// Params should include "action" key with specific action name.
    Custom,
}

impl Operation {
    /// The DIMSE operation name for this operation, as carried in the
    /// `x-dimse-op` header between Harmony instances.
    ///
    /// Returns `None` for operations that have no DIMSE service.
    pub fn dimse_op(&self) -> Option<&'static str> {
        match self {
            Operation::Query => Some("find"),
            Operation::Retrieve => Some("get"),
            Operation::Move => Some("move"),
            Operation::Store => Some("store"),
            Operation::Ping => Some("echo"),
            Operation::Update
            | Operation::Patch
            | Operation::Delete
            | Operation::Subscribe
            | Operation::Publish
            | Operation::Custom => None,
        }
    }
}

impl std::fmt::Display for Operation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Operation::Query => write!(f, "query"),
            Operation::Retrieve => write!(f, "retrieve"),
            Operation::Move => write!(f, "move"),
            Operation::Store => write!(f, "store"),
            Operation::Update => write!(f, "update"),
            Operation::Patch => write!(f, "patch"),
            Operation::Delete => write!(f, "delete"),
            Operation::Ping => write!(f, "ping"),
            Operation::Subscribe => write!(f, "subscribe"),
            Operation::Publish => write!(f, "publish"),
            Operation::Custom => write!(f, "custom"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_new() {
        let cmd = Command::new(Operation::Query);
        assert_eq!(cmd.operation, Operation::Query);
        assert!(cmd.params.is_empty());
        assert!(cmd.source.is_none());
    }

    #[test]
    fn test_command_with_source() {
        let cmd = Command::new(Operation::Store).with_source("test_transform");
        assert_eq!(cmd.source, Some("test_transform".to_string()));
    }

    #[test]
    fn test_command_with_param() {
        let cmd = Command::new(Operation::Query)
            .with_param("query_level", serde_json::json!("STUDY"))
            .with_param("limit", serde_json::json!(100));

        assert_eq!(
            cmd.params.get("query_level"),
            Some(&serde_json::json!("STUDY"))
        );
        assert_eq!(cmd.params.get("limit"), Some(&serde_json::json!(100)));
    }

    #[test]
    fn test_command_serialization() {
        let cmd = Command::new(Operation::Query)
            .with_source("test")
            .with_param("level", serde_json::json!("STUDY"));

        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("\"operation\":\"query\""));
        assert!(json.contains("\"source\":\"test\""));

        let parsed: Command = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.operation, Operation::Query);
        assert_eq!(parsed.source, Some("test".to_string()));
    }

    #[test]
    fn test_command_deserialization_minimal() {
        let json = r#"{"operation":"retrieve"}"#;
        let cmd: Command = serde_json::from_str(json).unwrap();
        assert_eq!(cmd.operation, Operation::Retrieve);
        assert!(cmd.params.is_empty());
        assert!(cmd.source.is_none());
    }

    #[test]
    fn test_operation_display() {
        assert_eq!(format!("{}", Operation::Query), "query");
        assert_eq!(format!("{}", Operation::Retrieve), "retrieve");
        assert_eq!(format!("{}", Operation::Store), "store");
        assert_eq!(format!("{}", Operation::Ping), "ping");
        assert_eq!(format!("{}", Operation::Custom), "custom");
    }

    #[test]
    fn test_dimse_op_names() {
        assert_eq!(Operation::Query.dimse_op(), Some("find"));
        assert_eq!(Operation::Retrieve.dimse_op(), Some("get"));
        assert_eq!(Operation::Move.dimse_op(), Some("move"));
        assert_eq!(Operation::Store.dimse_op(), Some("store"));
        assert_eq!(Operation::Ping.dimse_op(), Some("echo"));
    }

    #[test]
    fn test_dimse_op_is_none_without_a_dimse_service() {
        for op in [
            Operation::Update,
            Operation::Patch,
            Operation::Delete,
            Operation::Subscribe,
            Operation::Publish,
            Operation::Custom,
        ] {
            assert_eq!(op.dimse_op(), None, "{} has no DIMSE service", op);
        }
    }

    #[test]
    fn test_all_operations_serialize() {
        let operations = vec![
            Operation::Query,
            Operation::Retrieve,
            Operation::Move,
            Operation::Store,
            Operation::Update,
            Operation::Patch,
            Operation::Delete,
            Operation::Ping,
            Operation::Subscribe,
            Operation::Publish,
            Operation::Custom,
        ];

        for op in operations {
            let cmd = Command::new(op);
            let json = serde_json::to_string(&cmd).unwrap();
            let parsed: Command = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed.operation, op);
        }
    }
}
