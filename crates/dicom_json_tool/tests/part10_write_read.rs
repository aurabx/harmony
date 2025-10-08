use dicom_json_tool as tool;
use serde_json::json;
use std::path::PathBuf;

#[test]
fn write_then_read_part10_roundtrip() {
    // Build an identifier JSON
    let identifier = json!({
        "00100020": { "vr": "LO", "Value": ["XYZ123"] },
        "00100010": { "vr": "PN", "Value": [{"Alphabetic": "ALPHA^BETA"}] }
    });

    // Convert to object
    let obj = tool::json_value_to_identifier(&identifier).expect("json->identifier");

    // Write to ./tmp to respect user preference
    let mut out = PathBuf::from("./tmp/dicom");
    std::fs::create_dir_all(&out).expect("create tmp dir");
    out.push("roundtrip_test.dcm");

    tool::write_part10(&out, &obj).expect("write part 10");

    // Read back and compare
    let reopened = dicom_object::open_file(&out).expect("open written file");
    let back = tool::identifier_to_json_value(&reopened).expect("identifier->json");

    assert_eq!(back["00100020"]["Value"][0], "XYZ123");
    assert_eq!(back["00100010"]["Value"][0]["Alphabetic"], "ALPHA^BETA");
}
