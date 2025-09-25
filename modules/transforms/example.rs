let input = serde_json::json!({
    "PatientID": "12345",
    "StudyInstanceUID": "1.2.3.4.5"
});

let plan = TransformPlan {
name: "dicom_to_jmix".into(),
version: 1,
ops: vec![
    TransformOp::Map { from: "PatientID".into(), to: "patient.identifier".into() },
    TransformOp::TransformFn { from: "StudyInstanceUID".into(),
        to: "study.uid".into(),
        func: "prefix:urn:dicom:".into() },
],
};

let engine = TransformEngine::new(&registry);
let output = engine.apply(&plan, &input);

println!("{}", output);
// {
//   "patient": { "identifier": "12345" },
//   "study": { "uid": "urn:dicom:1.2.3.4.5" }
// }
