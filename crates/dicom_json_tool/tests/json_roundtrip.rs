use dicom_json_tool as tool;
use serde_json::{json, Value};

#[test]
fn roundtrip_simple_identifier() {
    // Sample DICOM JSON with PatientID (0010,0020) LO and PatientName (0010,0010) PN
    let input = json!({
        "00100020": { "vr": "LO", "Value": ["12345"] },
        "00100010": { "vr": "PN", "Value": [{"Alphabetic": "DOE^JOHN"}] }
    });

    let obj = tool::json_value_to_identifier(&input).expect("json->identifier");
    let out = tool::identifier_to_json_value(&obj).expect("identifier->json");

    // Values should be preserved
    assert_eq!(out["00100020"]["vr"], "LO");
    assert_eq!(out["00100020"]["Value"][0], "12345");

    assert_eq!(out["00100010"]["vr"], "PN");
    assert_eq!(out["00100010"]["Value"][0]["Alphabetic"], "DOE^JOHN");
}

#[test]
fn wrapper_roundtrip_snake_case() {
    let identifier = json!({
        "00100020": { "vr": "LO", "Value": ["ABC"] }
    });

    let cmd = tool::model::CommandMeta {
        message_id: Some(1),
        sop_class_uid: Some("1.2.840.10008.5.1.4.1.2.1.1".into()),
        priority: Some("MEDIUM".into()),
        direction: Some("REQUEST".into()),
    };

    let w = tool::wrap_with_command(identifier.clone(), Some(cmd.clone()), None);
    let id_ref = tool::unwrap_identifier(&w);

    assert_eq!(id_ref["00100020"]["Value"][0], "ABC");

    // Ensure we serialize with snake_case keys
    let w_json = serde_json::to_value(&w).expect("serialize wrapper");
    assert_eq!(w_json["command"]["message_id"], 1);
    assert_eq!(w_json["command"]["sop_class_uid"], "1.2.840.10008.5.1.4.1.2.1.1");
}

#[test]
fn wrapper_parse_user_format_snake_case() {
    // User-provided example with snake_case keys
    let user: Value = json!({
      "command": {
        "message_id": 1,
        "sop_class_uid": "1.2.840.10008.5.1.4.1.2.1.1",
        "priority": "MEDIUM",
        "direction": "REQUEST"
      },
      "identifier": {
        "00100020": {"vr": "LO", "Value": ["12345"]},
        "00100010": {"vr": "PN", "Value": [{"Alphabetic": "DOE*"}]},
        "00080020": {"vr": "DA", "Value": ["20240101-20241231"]},
        "00080050": {"vr": "SH", "Value": []}
      },
      "query_metadata": {
        "00100010": {"match_type": "WILDCARD"},
        "00080020": {"match_type": "RANGE"},
        "00080050": {"match_type": "RETURN_KEY"}
      }
    });

    let (cmd_opt, identifier, qmeta_opt) = tool::parse_wrapper_or_identifier(&user);
    let cmd = cmd_opt.expect("command meta present");
    assert_eq!(cmd.message_id, Some(1));
    assert_eq!(cmd.sop_class_uid.as_deref(), Some("1.2.840.10008.5.1.4.1.2.1.1"));
    assert!(identifier.get("00100020").is_some());
    let qmeta = qmeta_opt.expect("query metadata present");
    assert!(qmeta.0.get("00100010").is_some());
}
