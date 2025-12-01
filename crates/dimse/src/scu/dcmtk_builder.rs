//! DCMTK command argument builder for SCU operations

use std::collections::HashMap;
use std::path::PathBuf;

use crate::common::query_utils;
use crate::config::RemoteNode;
use crate::types::{FindQuery, GetQuery, MoveQuery, QueryLevel};

/// Builder for constructing DCMTK command-line arguments
pub struct DcmtkCommandBuilder {
    local_aet: String,
    storage_dir: PathBuf,
}

impl DcmtkCommandBuilder {
    /// Create a new DCMTK command builder
    pub fn new(local_aet: String, storage_dir: PathBuf) -> Self {
        Self {
            local_aet,
            storage_dir,
        }
    }

    /// Get the storage directory
    pub fn storage_dir(&self) -> &PathBuf {
        &self.storage_dir
    }

    /// Build base arguments common to all commands (AET, host, port)
    fn build_base_args(&self, node: &RemoteNode) -> Vec<String> {
        vec![
            "-aet".into(),
            self.local_aet.clone(),
            "-aec".into(),
            node.ae_title.clone(),
        ]
    }

    /// Add query level parameter to arguments
    fn add_query_level(
        &self,
        args: &mut Vec<String>,
        level: QueryLevel,
        command: &str,
    ) {
        let level_str = query_utils::query_level_to_string(level);

        match command {
            "find" => {
                args.push("-k".into());
                args.push(format!("QueryRetrieveLevel={}", level_str));
            }
            "move" => {
                args.push("-k".into());
                args.push(format!("0008,0052={}", level_str));
            }
            "get" => {
                args.push("-k".into());
                args.push(format!("QueryRetrieveLevel={}", level_str));
            }
            _ => {}
        }
    }

    /// Add query parameters to arguments with tag normalization
    fn add_query_params(
        &self,
        args: &mut Vec<String>,
        params: &HashMap<String, String>,
    ) {
        for (k, v) in params {
            let tag = query_utils::normalize_tag(k);
            args.push("-k".into());
            if v.is_empty() {
                args.push(format!("{}=", tag));
            } else {
                args.push(format!("{}={}", tag, v));
            }
        }
    }

    /// Build arguments for echoscu command
    pub fn build_echo_args(&self, node: &RemoteNode) -> Vec<String> {
        let mut args = self.build_base_args(node);
        args.push(node.host.clone());
        args.push(node.port.to_string());
        args
    }

    /// Build arguments for findscu command
    pub fn build_find_args(&self, node: &RemoteNode, query: &FindQuery) -> Vec<String> {
        let mut args = vec![
            "-P".into(), // Use Patient Root (default) unless specified otherwise
        ];
        args.extend(self.build_base_args(node));

        self.add_query_level(&mut args, query.query_level, "find");
        self.add_query_params(&mut args, &query.parameters);

        // Host and port at the end
        args.push(node.host.clone());
        args.push(node.port.to_string());

        args
    }

    /// Build arguments for movescu command
    pub fn build_move_args(
        &self,
        node: &RemoteNode,
        query: &MoveQuery,
        external_store_scp: bool,
        incoming_store_port: u16,
    ) -> Vec<String> {
        let mut args = vec![
            "-d".into(), // Enable verbose output for diagnostics
            "-S".into(), // Use Study Root query model for C-MOVE
        ];
        args.extend(self.build_base_args(node));

        // Move destination AET
        args.push("-aem".into());
        args.push(query.destination_aet.clone());

        self.add_query_level(&mut args, query.query_level, "move");
        self.add_query_params(&mut args, &query.parameters);

        // Incoming C-STORE handling
        // If using an external persistent Store SCP, do not open a transient listener (+P)
        if !external_store_scp {
            args.push("+P".into());
            args.push(incoming_store_port.to_string());
        }

        // Host and port at the end
        args.push(node.host.clone());
        args.push(node.port.to_string());

        args
    }

    /// Build arguments for getscu command
    pub fn build_get_args(&self, node: &RemoteNode, query: &GetQuery) -> Vec<String> {
        let mut args = Vec::new();

        // Use Patient Root by default or Study Root as per query level
        match query.query_level {
            QueryLevel::Patient => args.push("-P".into()),
            QueryLevel::Study | QueryLevel::Series | QueryLevel::Image => {
                args.push("-S".into())
            }
        }

        args.extend(self.build_base_args(node));

        self.add_query_level(&mut args, query.query_level, "get");
        self.add_query_params(&mut args, &query.parameters);

        // Host and port at the end
        args.push(node.host.clone());
        args.push(node.port.to_string());

        args
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_echo_args() {
        let builder = DcmtkCommandBuilder::new(
            "TEST_SCU".to_string(),
            PathBuf::from("/tmp"),
        );
        let node = RemoteNode::new("TEST_AET", "localhost", 11112);
        let args = builder.build_echo_args(&node);

        assert!(args.contains(&"-aet".to_string()));
        assert!(args.contains(&"TEST_SCU".to_string()));
        assert!(args.contains(&"-aec".to_string()));
        assert!(args.contains(&"TEST_AET".to_string()));
        assert!(args.contains(&"localhost".to_string()));
        assert!(args.contains(&"11112".to_string()));
    }

    #[test]
    fn test_build_find_args() {
        let builder = DcmtkCommandBuilder::new(
            "TEST_SCU".to_string(),
            PathBuf::from("/tmp"),
        );
        let node = RemoteNode::new("TEST_AET", "localhost", 11112);
        let query = FindQuery::patient(Some("12345".to_string()));
        let args = builder.build_find_args(&node, &query);

        assert!(args.contains(&"-P".to_string()));
        assert!(args.contains(&"-k".to_string()));
        assert!(args.iter().any(|a| a.contains("QueryRetrieveLevel=PATIENT")));
        assert!(args.iter().any(|a| a.contains("PatientID=12345")));
    }
}
