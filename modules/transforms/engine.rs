pub struct TransformEngine<'a> {
    registry: &'a TransformRegistry,
}

impl<'a> TransformEngine<'a> {
    pub fn new(registry: &'a TransformRegistry) -> Self {
        Self { registry }
    }

    pub fn apply(&self, plan: &TransformPlan, input: &Value) -> Value {
        let mut output = serde_json::Map::new();

        for op in &plan.ops {
            match op {
                TransformOp::Map { from, to } => {
                    if let Some(val) = input.pointer(&json_pointer(from)) {
                        set_pointer(&mut output, to, val.clone());
                    }
                }

                TransformOp::Const { to, value } => {
                    set_pointer(&mut output, to, value.clone());
                }

                TransformOp::TransformFn { from, to, func } => {
                    if let Some(val) = input.pointer(&json_pointer(from)) {
                        if let Some(f) = self.registry.get(func) {
                            if let Some(new_val) = f(val) {
                                set_pointer(&mut output, to, new_val);
                            }
                        }
                    }
                }

                TransformOp::Conditional { from, to, condition } => {
                    if condition == "exists" && input.pointer(&json_pointer(from)).is_some() {
                        if let Some(val) = input.pointer(&json_pointer(from)) {
                            set_pointer(&mut output, to, val.clone());
                        }
                    }
                }
            }
        }

        Value::Object(output)
    }
}
