use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(tag = "type")] // "map", "const", "transform", "conditional"
pub enum RawMapping {
    #[serde(rename = "map")]
    Map { from: String, to: String },

    #[serde(rename = "const")]
    Const { to: String, value: Value },

    #[serde(rename = "transform")]
    Transform { from: String, to: String, func: String },

    #[serde(rename = "conditional")]
    Conditional { from: String, to: String, condition: String },
}

#[derive(Debug, Deserialize)]
pub struct RawPlan {
    pub name: String,
    pub version: u32,
    pub mappings: Vec<RawMapping>,
}

impl TryFrom<RawPlan> for TransformPlan {
    type Error = String;

    fn try_from(raw: RawPlan) -> Result<Self, Self::Error> {
        let mut ops = Vec::new();

        for m in raw.mappings {
            match m {
                RawMapping::Map { from, to } => {
                    if from.is_empty() || to.is_empty() {
                        return Err("map requires 'from' and 'to'".into());
                    }
                    ops.push(TransformOp::Map { from, to });
                }

                RawMapping::Const { to, value } => {
                    if to.is_empty() {
                        return Err("const requires 'to'".into());
                    }
                    ops.push(TransformOp::Const { to, value });
                }

                RawMapping::Transform { from, to, func } => {
                    if from.is_empty() || to.is_empty() || func.is_empty() {
                        return Err("transform requires 'from', 'to', 'func'".into());
                    }
                    ops.push(TransformOp::TransformFn { from, to, func });
                }

                RawMapping::Conditional { from, to, condition } => {
                    if from.is_empty() || to.is_empty() || condition.is_empty() {
                        return Err("conditional requires 'from', 'to', 'condition'".into());
                    }
                    ops.push(TransformOp::Conditional { from, to, condition });
                }
            }
        }

        Ok(TransformPlan {
            name: raw.name,
            version: raw.version,
            ops,
        })
    }
}

// use serde_yaml;
//
// let yaml = r#"
// name: dicom_to_jmix
// version: 1
// mappings:
//   - type: map
//     from: PatientID
//     to: patient.identifier
//   - type: transform
//     from: StudyInstanceUID
//     to: study.uid
//     func: prefix:urn:dicom:
// "#;
//
// let raw_plan: RawPlan = serde_yaml::from_str(yaml).unwrap();
// let plan: TransformPlan = raw_plan.try_into().unwrap();
