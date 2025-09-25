use serde_json::Value;
use std::collections::HashMap;

/// A single transformation operator in a plan.
#[derive(Debug, Clone)]
pub enum TransformOp {
    /// Copy from -> to
    Map { from: String, to: String },

    /// Inject a constant
    Const { to: String, value: Value },

    /// Apply a registered function to a field
    TransformFn { from: String, to: String, func: String },

    /// Conditional mapping
    Conditional {
        from: String,
        to: String,
        condition: String, // e.g. "exists" or a named predicate
    },
}

/// A transform plan = ordered list of ops.
#[derive(Debug, Clone)]
pub struct TransformPlan {
    pub ops: Vec<TransformOp>,
    pub name: String,
    pub version: u32,
}

/// Convert dotted path ("PatientID") into JSON Pointer string ("/PatientID")
fn json_pointer(path: &str) -> String {
    format!("/{}", path.replace('.', "/"))
}

/// Set nested value in JSON Map by dotted path
fn set_pointer(map: &mut serde_json::Map<String, Value>, path: &str, val: Value) {
    let parts: Vec<&str> = path.split('.').collect();
    let mut current = map;

    for (i, part) in parts.iter().enumerate() {
        if i == parts.len() - 1 {
            current.insert(part.to_string(), val.clone());
        } else {
            current = current
                .entry(part.to_string())
                .or_insert_with(|| Value::Object(Default::default()))
                .as_object_mut()
                .unwrap();
        }
    }
}
