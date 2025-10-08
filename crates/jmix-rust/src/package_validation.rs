use crate::{assertion::AssertionManager, encryption::DecryptionManager, error::{JmixError, JmixResult}, types::{Audit, Files, Manifest, Metadata}, validation::SchemaValidator};
use sha2::{Digest, Sha256};
use std::{fs, path::{Path, PathBuf}};

#[derive(Debug, Clone, Default)]
pub struct ValidationOptions {
    pub schema_dir: Option<String>,
    pub validate_schema: bool,
    pub verify_assertions: bool,
    pub recipient_secret_key_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Default)]
pub struct ValidationReport {
    pub schema_ok: Option<bool>,
    pub payload_hash_ok: Option<bool>,
    pub assertions_ok: Option<bool>,
    pub encryption_ok: Option<bool>,
    pub errors: Vec<String>,
}

pub fn validate_package(package_dir: &Path, opts: &ValidationOptions) -> JmixResult<ValidationReport> {
    let mut report = ValidationReport::default();

    // Load manifest
    let manifest_path = package_dir.join("manifest.json");
    if !manifest_path.exists() {
        return Err(JmixError::Other(format!("manifest.json not found at {}", manifest_path.display())));
    }

    let manifest_str = fs::read_to_string(&manifest_path)?;
    let manifest: Manifest = serde_json::from_str(&manifest_str).map_err(JmixError::Json)?;

    // Load audit
    let audit_path = package_dir.join("audit.json");
    let audit: Option<Audit> = if audit_path.exists() {
        Some(serde_json::from_str(&fs::read_to_string(&audit_path)?)
            .map_err(JmixError::Json)?)
    } else { None };

    // Determine encryption
    let is_encrypted = manifest.security.encryption.is_some();

    // Schema validation (optional)
    if opts.validate_schema {
        let validator = if let Some(dir) = &opts.schema_dir { SchemaValidator::new(Some(dir.clone())) } else { SchemaValidator::with_default_config() };
        let mut schema_ok = true;
        if let Err(e) = validator.validate_manifest(&manifest) { schema_ok = false; report.errors.push(format!("manifest schema: {}", e)); }
        if let Some(a) = &audit {
            if let Err(e) = validator.validate_audit(a) { schema_ok = false; report.errors.push(format!("audit schema: {}", e)); }
        }
        // Load and validate payload/metadata.json and payload/files.json if unencrypted
        if !is_encrypted {
            let payload_dir = package_dir.join("payload");
            let metadata_path = payload_dir.join("metadata.json");
            if metadata_path.exists() {
                match serde_json::from_str::<Metadata>(&fs::read_to_string(&metadata_path)?) {
                    Ok(metadata) => {
                        if let Err(e) = validator.validate_metadata(&metadata) { schema_ok = false; report.errors.push(format!("metadata schema: {}", e)); }
                    }
                    Err(e) => { schema_ok = false; report.errors.push(format!("metadata parse: {}", e)); }
                }
            }
            let files_path = payload_dir.join("files.json");
            if files_path.exists() {
                match serde_json::from_str::<Files>(&fs::read_to_string(&files_path)?) {
                    Ok(files) => {
                        if let Err(e) = validator.validate_files(&files) { schema_ok = false; report.errors.push(format!("files schema: {}", e)); }
                    }
                    Err(e) => { schema_ok = false; report.errors.push(format!("files parse: {}", e)); }
                }
            }
        }
        report.schema_ok = Some(schema_ok);
    }

    // Payload hash verification
    if is_encrypted {
        // Need to decrypt to recompute payload hash
        match (&manifest.security.encryption, &opts.recipient_secret_key_path) {
            (Some(enc_info), Some(secret_path)) => {
                let payload_enc_path = package_dir.join("payload.enc");
                if !payload_enc_path.exists() {
                    report.errors.push("payload.enc missing for encrypted package".to_string());
                    report.encryption_ok = Some(false);
                    report.payload_hash_ok = Some(false);
                } else {
                    let ciphertext = fs::read(&payload_enc_path)?;
                    let dec = DecryptionManager::from_secret_key_file(secret_path)
                        .map_err(|e| JmixError::Other(format!("Failed to create decryption manager: {}", e)))?;
                    match dec.decrypt(&ciphertext, enc_info) {
                        Ok(plaintext_tar) => {
                            report.encryption_ok = Some(true);
                            // hash = sha256 of plaintext tar bytes
                            let mut hasher = Sha256::new();
                            hasher.update(&plaintext_tar);
                            let hash = format!("sha256:{:x}", hasher.finalize());
                            let ok = hash == manifest.security.payload_hash;
                            if !ok { report.errors.push("payload hash mismatch (encrypted)".to_string()); }
                            report.payload_hash_ok = Some(ok);
                        }
                        Err(e) => {
                            report.encryption_ok = Some(false);
                            report.payload_hash_ok = Some(false);
                            report.errors.push(format!("decryption failed: {}", e));
                        }
                    }
                }
            }
            (Some(_), None) => {
                // Cannot verify without key
                report.encryption_ok = None;
                report.payload_hash_ok = None;
            }
            _ => {}
        }
    } else {
        // Unencrypted: recompute hash over payload directory deterministically
        let payload_dir = package_dir.join("payload");
        if payload_dir.exists() {
            let ok = match compute_payload_hash_for_dir(&payload_dir) {
                Ok(hash) => {
                    let eq = hash == manifest.security.payload_hash;
                    if !eq { report.errors.push("payload hash mismatch (unencrypted)".to_string()); }
                    eq
                }
                Err(e) => { report.errors.push(format!("payload hash compute error: {}", e)); false }
            };
            report.payload_hash_ok = Some(ok);
        } else {
            report.payload_hash_ok = Some(false);
            report.errors.push("payload/ directory missing".to_string());
        }
    }

    // Assertions verification (optional)
    if opts.verify_assertions {
        let mut ok = true;
        // sender
        if let Some(assertion) = &manifest.sender.assertion {
            match AssertionManager::verify_assertion(assertion, &manifest.sender, &manifest) {
                Ok(crate::assertion::VerificationResult::Valid { .. }) => {}
                Ok(_) => { ok = false; report.errors.push("sender assertion invalid".to_string()); }
                Err(e) => { ok = false; report.errors.push(format!("sender assertion error: {}", e)); }
            }
        }
        // requester
        if let Some(requester) = &manifest.requester {
            if let Some(assertion) = &requester.assertion {
                match AssertionManager::verify_assertion(assertion, requester, &manifest) {
                    Ok(crate::assertion::VerificationResult::Valid { .. }) => {}
                    Ok(_) => { ok = false; report.errors.push("requester assertion invalid".to_string()); }
                    Err(e) => { ok = false; report.errors.push(format!("requester assertion error: {}", e)); }
                }
            }
        }
        // receivers
        for receiver in &manifest.receiver {
            if let Some(assertion) = &receiver.assertion {
                match AssertionManager::verify_assertion(assertion, receiver, &manifest) {
                    Ok(crate::assertion::VerificationResult::Valid { .. }) => {}
                    Ok(_) => { ok = false; report.errors.push("receiver assertion invalid".to_string()); }
                    Err(e) => { ok = false; report.errors.push(format!("receiver assertion error: {}", e)); }
                }
            }
        }
        report.assertions_ok = Some(ok);
    }

    Ok(report)
}

/// Deterministic payload directory hash used for unencrypted packages
fn compute_payload_hash_for_dir<P: AsRef<Path>>(payload_dir: P) -> JmixResult<String> {
    use walkdir::WalkDir;
    let payload_dir = payload_dir.as_ref();
    let mut paths: Vec<PathBuf> = Vec::new();
    for entry in WalkDir::new(payload_dir).into_iter().filter_map(|e| e.ok()) {
        if entry.file_type().is_file() {
            paths.push(entry.path().to_path_buf());
        }
    }
    paths.sort_by(|a, b| {
        let ra = a.strip_prefix(payload_dir).unwrap_or(a);
        let rb = b.strip_prefix(payload_dir).unwrap_or(b);
        ra.as_os_str().to_string_lossy().cmp(&rb.as_os_str().to_string_lossy())
    });
    let mut hasher = Sha256::new();
    for abs_path in paths {
        let rel = abs_path.strip_prefix(payload_dir).unwrap_or(&abs_path);
        let rel_str = rel.as_os_str().to_string_lossy();
        hasher.update(rel_str.as_bytes());
        hasher.update(&[b'\n']);
        let data = fs::read(&abs_path)?;
        hasher.update(&data);
    }
    let hash = hasher.finalize();
    Ok(format!("sha256:{:x}", hash))
}