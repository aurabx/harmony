pub type TransformFn = fn(&Value) -> Option<Value>;

pub struct TransformRegistry {
    funcs: HashMap<String, TransformFn>,
}

impl TransformRegistry {
    pub fn new() -> Self {
        Self { funcs: HashMap::new() }
    }

    pub fn register(&mut self, name: &str, func: TransformFn) {
        self.funcs.insert(name.to_string(), func);
    }

    pub fn get(&self, name: &str) -> Option<&TransformFn> {
        self.funcs.get(name)
    }
}

// Example
// fn prefix_urn(val: &Value) -> Option<Value> {
//     val.as_str()
//         .map(|s| Value::String(format!("urn:dicom:{}", s)))
// }
//
// let mut registry = TransformRegistry::new();
// registry.register("prefix:urn:dicom:", prefix_urn);
