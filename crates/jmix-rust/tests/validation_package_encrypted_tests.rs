use jmix_rs::{builder::JmixBuilder, config::Config, encryption::KeyPair, validate_package, ValidationOptions};
use std::{fs, path::PathBuf};

#[test]
#[ignore = "Generates keys and encrypts/decrypts; requires environment to allow file IO"]
fn validate_encrypted_package_roundtrip() {
    // Prepare tmp dirs
    let base_out = PathBuf::from("./tmp");
    let _ = fs::create_dir_all(&base_out);
    let keys_dir = base_out.join("keys_encrypted_test");
    let _ = fs::create_dir_all(&keys_dir);

    // Generate recipient keypair and save to files
    let kp = KeyPair::generate();
    let secret_path = keys_dir.join("recipient_secret.key");
    let public_path = keys_dir.join("recipient_public.key");
    kp.save_to_files(&secret_path, &public_path).expect("save keys");

    // Minimal config
    let cfg = Config::default();

    // Build encrypted envelope
    let recipient_public_b64 = kp.public_key_base64();
    let builder = JmixBuilder::with_encryption_and_signing(&recipient_public_b64).expect("builder with enc");

    // Create small DICOM dir
    let dicom_dir = base_out.join("unit_test_dicom_enc");
    let _ = fs::create_dir_all(&dicom_dir);
    let _ = fs::write(dicom_dir.join("image_001.dcm"), b"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxDICM");

    let (envelope, dicom_files) = builder.build_from_dicom(&dicom_dir, &cfg).expect("build");
    let _saved = builder.save_to_files(&envelope, &dicom_files, &base_out).expect("save");

    // Envelope dir
    let env_dir = base_out.join(format!("{}.jmix", envelope.manifest.id));
    assert!(env_dir.exists());

    // Validate with decryption key
    let opts = ValidationOptions {
        validate_schema: false,
        verify_assertions: false,
        schema_dir: None,
        recipient_secret_key_path: Some(secret_path),
    };

    let report = validate_package(&env_dir, &opts).expect("validate");
    assert_eq!(report.encryption_ok, Some(true));
    assert_eq!(report.payload_hash_ok, Some(true));
}

#[test]
#[ignore = "Requires file IO and key generation; verifies wrong key fails decryption"]
fn validate_encrypted_wrong_key_fails() {
    use jmix_rs::encryption::KeyPair;

    let base_out = PathBuf::from("./tmp");
    let _ = fs::create_dir_all(&base_out);

    // Generate two different keypairs
    let kp_good = KeyPair::generate();
    let kp_wrong = KeyPair::generate();

    let keys_dir = base_out.join("keys_wrong_key_test");
    let _ = fs::create_dir_all(&keys_dir);
    let secret_wrong = keys_dir.join("wrong_secret.key");
    let public_good = keys_dir.join("good_public.key");
    kp_wrong.save_to_files(&secret_wrong, &keys_dir.join("wrong_public.key")).expect("save wrong");
    // Save good public to satisfy API (not strictly needed for validate)
    std::fs::File::create(&public_good).expect("touch");

    // Build encrypted envelope with good public key
    let recipient_public_b64 = kp_good.public_key_base64();
    let builder = JmixBuilder::with_encryption_and_signing(&recipient_public_b64).expect("builder with enc");

    let dicom_dir = base_out.join("unit_test_dicom_wrongkey");
    let _ = fs::create_dir_all(&dicom_dir);
    let _ = fs::write(dicom_dir.join("image_001.dcm"), b"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxDICM");

    let cfg = Config::default();
    let (envelope, dicom_files) = builder.build_from_dicom(&dicom_dir, &cfg).expect("build");
    let _saved = builder.save_to_files(&envelope, &dicom_files, &base_out).expect("save");

    let env_dir = base_out.join(format!("{}.jmix", envelope.manifest.id));

    // Validate using wrong secret key -> decryption should fail
    let opts = ValidationOptions {
        validate_schema: false,
        verify_assertions: false,
        schema_dir: None,
        recipient_secret_key_path: Some(secret_wrong),
    };
    let report = validate_package(&env_dir, &opts).expect("validate");
    assert_eq!(report.encryption_ok, Some(false));
    assert_eq!(report.payload_hash_ok, Some(false));
}
