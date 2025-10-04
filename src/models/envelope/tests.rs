#![cfg(test)]

use std::collections::HashMap;
use crate::models::envelope::envelope::{Envelope, RequestDetails};

#[test]
fn test_create_envelope() {
    let request_details = RequestDetails{
        method: "POST".to_string(),
        uri: "/example-path".to_string(),
        headers: {
            let mut headers = HashMap::new();
            headers.insert("Content-Type".to_string(), "application/json".to_string());
            headers
        },
        metadata: HashMap::new(),
    };

    let original_data = vec!["example", "data"];
    let envelope = Envelope::new(request_details.clone(), original_data);

    assert_eq!(envelope.request_details.method, request_details.method);
    assert!(envelope.normalized_data.is_some());
}