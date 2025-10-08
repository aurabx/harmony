use jmix_rs::{builder::JmixBuilder, config::Config, validate_package, ValidationOptions};
use std::{fs, path::PathBuf};

#[test]
fn validate_unencrypted_package_roundtrip() {
    // Prepare a temp-like output under ./tmp per project preference
    let base_out = PathBuf::from("./tmp");
    let _ = fs::create_dir_all(&base_out);

    // Create a tiny fake DICOM directory with files having DICM signature
    let dicom_dir = base_out.join("unit_test_dicom");
    let _ = fs::create_dir_all(&dicom_dir);
    let _ = fs::write(dicom_dir.join("image_001.dcm"), b"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxDICM");
    let _ = fs::write(dicom_dir.join("image_002.dcm"), b"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxDICM");

    // Minimal config
    let cfg = Config::default();

    // Build envelope
    let builder = JmixBuilder::new();
    let (envelope, dicom_files) = builder.build_from_dicom(&dicom_dir, &cfg).expect("build from dicom");

    // Save envelope
    let saved = builder
        .save_to_files(&envelope, &dicom_files, &base_out)
        .expect("save envelope");

    // Determine envelope directory path
    let env_dir = base_out.join(format!("{}.jmix", envelope.manifest.id));
    assert!(env_dir.exists(), "envelope dir should exist");

    // Validate without schema checks (no schema files in repo) and without assertions
    let opts = ValidationOptions { validate_schema: false, verify_assertions: false, schema_dir: None, recipient_secret_key_path: None };
    let report = validate_package(&env_dir, &opts).expect("validate");

    // Expect payload hash check success for unencrypted
    assert_eq!(report.payload_hash_ok, Some(true));
}

#[test]
fn test_unencrypted_payload_tamper_detected() {
    use std::io::Write;

    // Prepare output
    let base_out = PathBuf::from("./tmp");
    let _ = fs::create_dir_all(&base_out);

    // Build small unencrypted envelope
    let dicom_dir = base_out.join("unit_test_dicom_tamper");
    let _ = fs::create_dir_all(&dicom_dir);
    let _ = fs::write(dicom_dir.join("image_001.dcm"), b"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxDICM");

    let cfg = Config::default();
    let builder = JmixBuilder::new();
    let (envelope, dicom_files) = builder.build_from_dicom(&dicom_dir, &cfg).expect("build");
    let _saved = builder.save_to_files(&envelope, &dicom_files, &base_out).expect("save");

    // Tamper metadata.json by appending a newline (changes bytes without breaking JSON)
    let env_dir = base_out.join(format!("{}.jmix", envelope.manifest.id));
    let metadata_path = env_dir.join("payload").join("metadata.json");
    let mut content = fs::read(&metadata_path).expect("read metadata");
    content.extend_from_slice(b"\n");
    let mut f = std::fs::OpenOptions::new().write(true).truncate(true).open(&metadata_path).expect("open");
    f.write_all(&content).expect("write");

    // Validate and expect hash mismatch
    let opts = ValidationOptions { validate_schema: false, verify_assertions: false, schema_dir: None, recipient_secret_key_path: None };
    let report = validate_package(&env_dir, &opts).expect("validate");
    assert_eq!(report.payload_hash_ok, Some(false));
}
