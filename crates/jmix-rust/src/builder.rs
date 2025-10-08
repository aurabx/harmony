use crate::{
    assertion::AssertionManager,
    config::Config,
    dicom::DicomProcessor,
    encryption::EncryptionManager,
    error::{JmixError, JmixResult},
    jws::JwsManager,
    types::*,
    validation::SchemaValidator,
};
use sha2::{Sha256, Digest};
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use uuid::Uuid;
use walkdir::WalkDir;

/// JMIX Builder for creating medical data envelopes
pub struct JmixBuilder {
    validator: Option<SchemaValidator>,
    jws_manager: Option<JwsManager>,
    encryption_manager: Option<EncryptionManager>,
    assertion_manager: Option<AssertionManager>,
}

impl JmixBuilder {
    /// Create a new JMIX builder
    pub fn new() -> Self {
        Self {
            validator: None,
            jws_manager: None,
            encryption_manager: None,
            assertion_manager: None,
        }
    }

    /// Create a new JMIX builder with validation enabled
    pub fn with_validation() -> JmixResult<Self> {
        Ok(Self {
            validator: Some(SchemaValidator::with_default_config()),
            jws_manager: None,
            encryption_manager: None,
            assertion_manager: None,
        })
    }
    
    /// Create a new JMIX builder with custom schema path
    pub fn with_schema_path<P: AsRef<Path>>(schema_path: P) -> JmixResult<Self> {
        Ok(Self {
            validator: Some(SchemaValidator::new(Some(schema_path.as_ref().to_string_lossy().to_string()))),
            jws_manager: None,
            encryption_manager: None,
            assertion_manager: None,
        })
    }
    
    /// Create a new JMIX builder with JWS signing enabled (generated key)
    pub fn with_signing() -> JmixResult<Self> {
        Ok(Self {
            validator: None,
            jws_manager: Some(JwsManager::with_generated_key()?),
            encryption_manager: None,
            assertion_manager: None,
        })
    }
    
    /// Create a new JMIX builder with JWS signing using a key file
    pub fn with_signing_key<P: AsRef<Path>>(key_path: P) -> JmixResult<Self> {
        Ok(Self {
            validator: None,
            jws_manager: Some(JwsManager::with_key_file(key_path)?),
            encryption_manager: None,
            assertion_manager: None,
        })
    }

    /// Create a new JMIX builder with both validation and signing
    pub fn with_validation_and_signing() -> JmixResult<Self> {
        Ok(Self {
            validator: Some(SchemaValidator::with_default_config()),
            jws_manager: Some(JwsManager::with_generated_key()?),
            encryption_manager: None,
            assertion_manager: None,
        })
    }
    
    /// Create a new JMIX builder with encryption enabled using recipient's public key
    pub fn with_encryption(recipient_public_key_b64: &str) -> JmixResult<Self> {
        let encryption_manager = EncryptionManager::from_base64_public_key(recipient_public_key_b64)
            .map_err(|e| JmixError::Other(format!("Failed to create encryption manager: {}", e)))?;
        Ok(Self {
            validator: None,
            jws_manager: None,
            encryption_manager: Some(encryption_manager),
            assertion_manager: None,
        })
    }
    
    /// Create a new JMIX builder with encryption enabled using recipient's public key file
    pub fn with_encryption_key_file<P: AsRef<Path>>(recipient_public_key_path: P) -> JmixResult<Self> {
        let encryption_manager = EncryptionManager::from_public_key_file(recipient_public_key_path)
            .map_err(|e| JmixError::Other(format!("Failed to create encryption manager: {}", e)))?;
        Ok(Self {
            validator: None,
            jws_manager: None,
            encryption_manager: Some(encryption_manager),
            assertion_manager: None,
        })
    }
    
    /// Create a new JMIX builder with encryption and signing enabled
    pub fn with_encryption_and_signing(recipient_public_key_b64: &str) -> JmixResult<Self> {
        let encryption_manager = EncryptionManager::from_base64_public_key(recipient_public_key_b64)
            .map_err(|e| JmixError::Other(format!("Failed to create encryption manager: {}", e)))?;
        Ok(Self {
            validator: None,
            jws_manager: Some(JwsManager::with_generated_key()?),
            encryption_manager: Some(encryption_manager),
            assertion_manager: None,
        })
    }
    
    /// Create a new JMIX builder with full features: validation, encryption, and signing
    pub fn with_full_security(recipient_public_key_b64: &str) -> JmixResult<Self> {
        let encryption_manager = EncryptionManager::from_base64_public_key(recipient_public_key_b64)
            .map_err(|e| JmixError::Other(format!("Failed to create encryption manager: {}", e)))?;
        Ok(Self {
            validator: Some(SchemaValidator::with_default_config()),
            jws_manager: Some(JwsManager::with_generated_key()?),
            encryption_manager: Some(encryption_manager),
            assertion_manager: None,
        })
    }
    
    /// Create a new JMIX builder with assertions enabled
    pub fn with_assertions() -> JmixResult<Self> {
        Ok(Self {
            validator: None,
            jws_manager: None,
            encryption_manager: None,
            assertion_manager: Some(AssertionManager::with_generated_key()?),
        })
    }
    
    /// Create a new JMIX builder with signing and assertions enabled
    pub fn with_signing_and_assertions() -> JmixResult<Self> {
        // For now, use separate key managers. In a real implementation,
        // you might want to coordinate the keys between JWS and assertion managers
        Ok(Self {
            validator: None,
            jws_manager: Some(JwsManager::with_generated_key()?),
            encryption_manager: None,
            assertion_manager: Some(AssertionManager::with_generated_key()?),
        })
    }
    
    /// Create a new JMIX builder with complete security: encryption, signing, and assertions
    pub fn with_complete_security(recipient_public_key_b64: &str) -> JmixResult<Self> {
        let encryption_manager = EncryptionManager::from_base64_public_key(recipient_public_key_b64)
            .map_err(|e| JmixError::Other(format!("Failed to create encryption manager: {}", e)))?;
        Ok(Self {
            validator: Some(SchemaValidator::with_default_config()),
            jws_manager: Some(JwsManager::with_generated_key()?),
            encryption_manager: Some(encryption_manager),
            assertion_manager: Some(AssertionManager::with_generated_key()?),
        })
    }

    /// Build a JMIX envelope from DICOM files
    pub fn build_from_dicom<P: AsRef<Path>>(
        &self,
        dicom_path: P,
        config: &Config,
    ) -> JmixResult<(Envelope, Vec<PathBuf>)> {
        // Process DICOM files
        let processor = DicomProcessor::new();
        let dicom_metadata = processor.process_dicom_folder(&dicom_path, Some(config))?;

        // Collect source DICOM files (we'll copy them later)
        let dicom_sources = self.collect_dicom_files(&dicom_path)?;

        // Generate unique ID for this envelope
        let envelope_id = Uuid::new_v4().to_string();
        let timestamp = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

        // Create manifest
        let manifest = self.create_manifest(config, &envelope_id, &timestamp)?;

        // Create metadata from DICOM and config
        let metadata = self.create_metadata(config, &envelope_id, &timestamp, &dicom_metadata)?;

        // Create audit trail
        let audit = self.create_audit(config, &envelope_id, &timestamp)?;

        // Create envelope
        let envelope = Envelope {
            manifest,
            metadata,
            audit,
            manifest_jws: None, // JWS signing not yet implemented
        };

        // Validate if validator is present
        if let Some(validator) = &self.validator {
            validator.validate_manifest(&envelope.manifest)?;
            validator.validate_metadata(&envelope.metadata)?;
            validator.validate_audit(&envelope.audit)?;
        }

        Ok((envelope, dicom_sources))
    }

    /// Save envelope components as a proper JMIX envelope structure
    pub fn save_to_files<P: AsRef<Path>>(
        &self,
        envelope: &Envelope,
        dicom_files: &[PathBuf],
        output_dir: P,
    ) -> JmixResult<Vec<PathBuf>> {
        if self.encryption_manager.is_some() {
            self.save_encrypted_envelope(envelope, dicom_files, output_dir)
        } else {
            self.save_unencrypted_envelope(envelope, dicom_files, output_dir)
        }
    }
    
    /// Save unencrypted envelope (original behavior)
    fn save_unencrypted_envelope<P: AsRef<Path>>(
        &self,
        envelope: &Envelope,
        dicom_files: &[PathBuf],
        output_dir: P,
    ) -> JmixResult<Vec<PathBuf>> {
        let base_dir = output_dir.as_ref();
        
        // Create JMIX envelope directory: <id>.jmix
        let envelope_dir = base_dir.join(format!("{}.jmix", envelope.manifest.id));
        if !envelope_dir.exists() {
            fs::create_dir_all(&envelope_dir)?;
        }

        // Create payload subdirectory
        let payload_dir = envelope_dir.join("payload");
        if !payload_dir.exists() {
            fs::create_dir_all(&payload_dir)?;
        }

        // Create payload subdirectories
        let dicom_dir = payload_dir.join("dicom");
        let files_dir = payload_dir.join("files");
        fs::create_dir_all(&dicom_dir)?;
        fs::create_dir_all(&files_dir)?;

        let mut saved_files = Vec::new();

        // Note: manifest.json will be written after payload is prepared and payload hash is computed

        // Save audit.json at root level
        let audit_path = envelope_dir.join("audit.json");
        let audit_json = serde_json::to_string_pretty(&envelope.audit)
            .map_err(|e| JmixError::Json(e))?;
        fs::write(&audit_path, audit_json)?;
        saved_files.push(audit_path);

        // Save metadata.json in payload/
        let metadata_path = payload_dir.join("metadata.json");
        let metadata_json = serde_json::to_string_pretty(&envelope.metadata)
            .map_err(|e| JmixError::Json(e))?;
        fs::write(&metadata_path, metadata_json)?;
        saved_files.push(metadata_path.clone());

        // Create and save files.json in payload/ (with real hashes)
        let mut files = self.create_files_manifest(&envelope, dicom_files)?;
        
        // Update metadata.json hash now that it's written
        if let Some(metadata_entry) = files.iter_mut().find(|f| f.file == "metadata.json") {
            metadata_entry.hash = self.sha256_file(&metadata_path).ok();
            metadata_entry.size_bytes = fs::metadata(&metadata_path).ok().map(|m| m.len() as i64);
        }
        
        let files_path = payload_dir.join("files.json");
        let files_json = serde_json::to_string_pretty(&files)
            .map_err(|e| JmixError::Json(e))?;
        fs::write(&files_path, files_json)?;
        saved_files.push(files_path);

        // Copy DICOM files to payload/dicom/
        for (i, source_path) in dicom_files.iter().enumerate() {
            if let Some(file_name) = source_path.file_name() {
                let dest_path = dicom_dir.join(file_name);
                fs::copy(source_path, &dest_path)
                    .map_err(|e| JmixError::Io(e))?;
                saved_files.push(dest_path);
                
                if i % 5 == 0 {  // Progress indicator for large sets
                    println!("     Copied DICOM file {}/{}: {}", i + 1, dicom_files.len(), file_name.to_string_lossy());
                }
            }
        }
        
        if !dicom_files.is_empty() {
            println!("     ✓ Copied {} DICOM files to payload/dicom/", dicom_files.len());
        }

        // Compute deterministic payload hash over payload directory contents
        let payload_hash = self.compute_payload_hash_for_dir(&payload_dir)?;

        // Save manifest.json at root level with computed payload hash
        let mut updated_envelope = envelope.clone();
        updated_envelope
            .manifest
            .security
            .payload_hash = payload_hash;

        let manifest_path = envelope_dir.join("manifest.json");
        let manifest_json = serde_json::to_string_pretty(&updated_envelope.manifest)
            .map_err(|e| JmixError::Json(e))?;
        fs::write(&manifest_path, &manifest_json)?;
        saved_files.push(manifest_path);

        // Generate manifest.jws if signing is enabled
        if let Some(jws_manager) = &self.jws_manager {
            let jws = jws_manager.sign_json(&manifest_json)?;
            let jws_path = envelope_dir.join("manifest.jws");
            fs::write(&jws_path, jws)?;
            saved_files.push(jws_path);
            println!("     ✓ Generated manifest.jws signature");
        }

        // Create README explaining the structure
        let readme_path = envelope_dir.join("README.md");
        let readme_content = format!(
            "# JMIX Envelope: {}\n\nThis is a JMIX envelope containing medical imaging data and metadata.\n\n## Structure\n\n- `manifest.json` - Envelope metadata (no PII)\n- `audit.json` - Audit trail of envelope events\n- `payload/` - Contains the actual medical data\n  - `metadata.json` - Patient and study metadata\n  - `files.json` - File manifest\n  - `dicom/` - DICOM files (empty - would contain .dcm files)\n  - `files/` - Additional files (empty - would contain PDFs, reports)\n\nGenerated by jmix-rs at {}\n",
            updated_envelope.manifest.id,
            updated_envelope.manifest.timestamp
        );
        fs::write(&readme_path, readme_content)?;
        saved_files.push(readme_path);

        Ok(saved_files)
    }
    
    /// Save encrypted envelope
    fn save_encrypted_envelope<P: AsRef<Path>>(
        &self,
        envelope: &Envelope,
        dicom_files: &[PathBuf],
        output_dir: P,
    ) -> JmixResult<Vec<PathBuf>> {
        let encryption_manager = self.encryption_manager.as_ref()
            .ok_or_else(|| JmixError::Other("Encryption manager not configured".to_string()))?;
        
        let base_dir = output_dir.as_ref();
        
        // Create JMIX envelope directory: <id>.jmix
        let envelope_dir = base_dir.join(format!("{}.jmix", envelope.manifest.id));
        if !envelope_dir.exists() {
            fs::create_dir_all(&envelope_dir)?;
        }
        
        let mut saved_files = Vec::new();
        
        // First, create the payload in memory/temp location
        let temp_payload_dir = tempfile::tempdir()
            .map_err(|e| JmixError::Other(format!("Failed to create temp dir: {}", e)))?;
        
        let payload_temp = temp_payload_dir.path();
        let dicom_temp = payload_temp.join("dicom");
        let files_temp = payload_temp.join("files");
        fs::create_dir_all(&dicom_temp)?;
        fs::create_dir_all(&files_temp)?;
        
        // Save metadata.json to temp payload
        let metadata_path = payload_temp.join("metadata.json");
        let metadata_json = serde_json::to_string_pretty(&envelope.metadata)
            .map_err(|e| JmixError::Json(e))?;
        fs::write(&metadata_path, metadata_json)?;
        
        // Create and save files.json in temp payload
        let mut files_manifest = self.create_files_manifest(&envelope, dicom_files)?;
        
        // Update metadata.json hash
        if let Some(metadata_entry) = files_manifest.iter_mut().find(|f| f.file == "metadata.json") {
            metadata_entry.hash = self.sha256_file(&metadata_path).ok();
            metadata_entry.size_bytes = fs::metadata(&metadata_path).ok().map(|m| m.len() as i64);
        }
        
        let files_path = payload_temp.join("files.json");
        let files_json = serde_json::to_string_pretty(&files_manifest)
            .map_err(|e| JmixError::Json(e))?;
        fs::write(&files_path, files_json)?;
        
        // Copy DICOM files to temp payload
        for source_path in dicom_files {
            if let Some(file_name) = source_path.file_name() {
                let dest_path = dicom_temp.join(file_name);
                fs::copy(source_path, &dest_path)?;
            }
        }
        
        // Create TAR archive of the payload directory
        let payload_tar_path = envelope_dir.join("payload.tar");
        self.create_tar_archive(payload_temp, &payload_tar_path)?;
        
        // Read the TAR archive to encrypt it
        let payload_data = fs::read(&payload_tar_path)?;
        
        // Compute deterministic payload hash over plaintext TAR (before encryption)
        let payload_hash = self.sha256_bytes(&payload_data);

        // Encrypt the payload
        let encryption_result = encryption_manager.encrypt(&payload_data)
            .map_err(|e| JmixError::Other(format!("Encryption failed: {}", e)))?;
        
        // Write encrypted payload
        let encrypted_payload_path = envelope_dir.join("payload.enc");
        fs::write(&encrypted_payload_path, &encryption_result.ciphertext)?;
        saved_files.push(encrypted_payload_path);
        
        // Remove unencrypted TAR file
        let _ = fs::remove_file(&payload_tar_path);
        
        // Create updated manifest with encryption info and payload hash
        let mut updated_envelope = envelope.clone();
        updated_envelope.manifest.security.encryption = Some(encryption_result.info);
        updated_envelope.manifest.security.payload_hash = payload_hash;
        
        // Save manifest.json with encryption info
        let manifest_path = envelope_dir.join("manifest.json");
        let manifest_json = serde_json::to_string_pretty(&updated_envelope.manifest)
            .map_err(|e| JmixError::Json(e))?;
        fs::write(&manifest_path, &manifest_json)?;
        saved_files.push(manifest_path);
        
        // Generate manifest.jws if signing is enabled
        if let Some(jws_manager) = &self.jws_manager {
            let jws = jws_manager.sign_json(&manifest_json)?;
            let jws_path = envelope_dir.join("manifest.jws");
            fs::write(&jws_path, jws)?;
            saved_files.push(jws_path);
            println!("     ✓ Generated manifest.jws signature");
        }
        
        // Save audit.json (unencrypted)
        let audit_path = envelope_dir.join("audit.json");
        let audit_json = serde_json::to_string_pretty(&envelope.audit)
            .map_err(|e| JmixError::Json(e))?;
        fs::write(&audit_path, audit_json)?;
        saved_files.push(audit_path);
        
        // Create README for encrypted envelope
        let readme_path = envelope_dir.join("README.md");
        let readme_content = format!(
            "# JMIX Encrypted Envelope: {}\n\nThis is an encrypted JMIX envelope containing medical imaging data and metadata.\n\n## Structure\n\n- `manifest.json` - Envelope metadata with encryption info (no PII)\n- `audit.json` - Audit trail of envelope events\n- `payload.enc` - Encrypted payload containing:\n  - `metadata.json` - Patient and study metadata\n  - `files.json` - File manifest\n  - `dicom/` - DICOM files\n  - `files/` - Additional files\n\nEncryption: AES-256-GCM with ECDH key agreement\nGenerated by jmix-rs at {}\n\nTo decrypt, use the recipient's private key with a JMIX-compatible decryption tool.\n",
            envelope.manifest.id,
            envelope.manifest.timestamp
        );
        fs::write(&readme_path, readme_content)?;
        saved_files.push(readme_path);
        
        println!("     ✓ Encrypted payload ({} bytes) as payload.enc", payload_data.len());
        if !dicom_files.is_empty() {
            println!("     ✓ Encrypted {} DICOM files in payload", dicom_files.len());
        }
        
        Ok(saved_files)
    }
    
    /// Create a TAR archive from a directory
    fn create_tar_archive<P: AsRef<Path>, Q: AsRef<Path>>(
        &self,
        source_dir: P,
        output_path: Q,
    ) -> JmixResult<()> {
        use tar::Builder;
        
        let tar_file = fs::File::create(&output_path)?;
        let mut tar_builder = Builder::new(tar_file);
        
        // Add all files from the source directory recursively
        tar_builder.append_dir_all(".", &source_dir)?;
        tar_builder.finish()?;
        
        Ok(())
    }

    /// Create manifest from config and envelope metadata
    fn create_manifest(
        &self,
        config: &Config,
        envelope_id: &str,
        timestamp: &str,
    ) -> JmixResult<Manifest> {
        self.create_manifest_with_encryption(config, envelope_id, timestamp, None)
    }
    
    /// Create manifest with optional encryption info
    fn create_manifest_with_encryption(
        &self,
        config: &Config,
        envelope_id: &str,
        timestamp: &str,
        encryption_info: Option<EncryptionInfo>,
    ) -> JmixResult<Manifest> {
        // Create a preliminary manifest for assertion generation
        let mut preliminary_manifest = self.create_preliminary_manifest(config, envelope_id, timestamp, encryption_info)?;
        
        // Generate assertions if assertion manager is present
        if let Some(assertion_manager) = &self.assertion_manager {
            // Generate sender assertion
            let sender_assertion_result = assertion_manager.create_sender_assertion(
                &preliminary_manifest.sender,
                &preliminary_manifest,
                None, // No expiration for now
                None, // No key reference for now
                None, // No directory attestation for now
            )?;
            preliminary_manifest.sender.assertion = Some(sender_assertion_result.assertion);
            
            // Generate requester assertion if requester exists
            if preliminary_manifest.requester.is_some() {
                // Clone the requester to avoid borrowing issues
                let requester_copy = preliminary_manifest.requester.as_ref().unwrap().clone();
                let requester_assertion_result = assertion_manager.create_requester_assertion(
                    &requester_copy,
                    &preliminary_manifest,
                    None, // No expiration
                    None, // No key reference
                    None, // No directory attestation
                )?;
                preliminary_manifest.requester.as_mut().unwrap().assertion = Some(requester_assertion_result.assertion);
            }

            // Generate receiver assertions for each receiver
            for (i, recv) in preliminary_manifest.receiver.clone().iter().enumerate() {
                let receiver_assertion = assertion_manager.create_receiver_assertion(
                    recv,
                    &preliminary_manifest,
                    i,
                    None,
                    None,
                    None,
                )?;
                if let Some(slot) = preliminary_manifest.receiver.get_mut(i) {
                    slot.assertion = Some(receiver_assertion.assertion);
                }
            }
        }
        
        Ok(preliminary_manifest)
    }
    
    /// Create preliminary manifest without assertions
    fn create_preliminary_manifest(
        &self,
        config: &Config,
        envelope_id: &str,
        timestamp: &str,
        encryption_info: Option<EncryptionInfo>,
    ) -> JmixResult<Manifest> {
        // Convert config entities to manifest entities
        let sender = Entity {
            name: Some(config.sender.name.clone()),
            id: config.sender.id.clone(),
            contact: self.convert_contact(&config.sender.contact),
            assertion: None, // Will be filled in later if assertion manager is present
        };

        let requester = if let Some(_req) = config.requester.assertion.as_ref() {
            Some(Entity {
                name: Some(config.requester.name.clone()),
                id: config.requester.id.clone(),
                contact: self.convert_contact(&config.requester.contact),
                assertion: None, // Convert later when assertion types are implemented
            })
        } else {
            Some(Entity {
                name: Some(config.requester.name.clone()),
                id: config.requester.id.clone(),
                contact: self.convert_contact(&config.requester.contact),
                assertion: None,
            })
        };

        let receiver: Vec<Entity> = config
            .receivers
            .iter()
            .map(|r| Entity {
                name: Some(r.name.clone()),
                id: r.id.clone(),
                contact: self.convert_contact(&r.contact),
                assertion: None, // Convert later when assertion types are implemented
            })
            .collect();

        // Create security section
        let security = Security {
            classification: config
                .security
                .as_ref()
                .and_then(|s| s.classification.clone()),
            payload_hash: "sha256:placeholder".to_string(), // Would be calculated from actual payload
            jws: None,
            signature: None,
            encryption: encryption_info,
        };

        // Create extensions
        let extensions = if config.custom_tags.is_some() || config.consent.is_some() || config.deid_keys.is_some() {
            Some(Extensions {
                custom_tags: config.custom_tags.clone(),
                consent: config.consent.as_ref().map(|c| ConsentExtension {
                    status: c.status.clone(),
                    scope: c.scope.clone(),
                    method: c.method.clone(),
                    signed_on: Some(timestamp.split('T').next().unwrap_or(timestamp).to_string()),
                }),
                deid: config.deid_keys.as_ref().map(|keys| DeidExtension {
                    keys: keys.clone(),
                }),
                additional: HashMap::new(),
            })
        } else {
            None
        };

        Ok(Manifest {
            version: config.version.clone().unwrap_or_else(|| "1.0".to_string()),
            id: envelope_id.to_string(),
            timestamp: timestamp.to_string(),
            sender,
            requester,
            receiver,
            security,
            extensions,
        })
    }

    /// Create metadata from config and DICOM metadata
    fn create_metadata(
        &self,
        config: &Config,
        envelope_id: &str,
        timestamp: &str,
        dicom_metadata: &crate::dicom::DicomMetadata,
    ) -> JmixResult<Metadata> {
        // Create patient information (prefer DICOM data, fallback to config)
        let patient_name = dicom_metadata
            .patient_name
            .as_ref()
            .or(config.patient.name.as_ref());
        
        let human_name = if let Some(name) = patient_name {
            // Parse name if it contains a comma (DICOM format: "Family, Given")
            if name.contains(',') {
                let parts: Vec<&str> = name.splitn(2, ',').collect();
                let family = parts[0].trim();
                let given = parts.get(1).map(|g| vec![g.trim().to_string()]);
                Some(HumanName {
                    family: if family.is_empty() { None } else { Some(family.to_string()) },
                    given,
                    prefix: None,
                    suffix: None,
                    text: Some(name.clone()),
                })
            } else {
                Some(HumanName {
                    family: None,
                    given: Some(vec![name.clone()]),
                    prefix: None,
                    suffix: None,
                    text: Some(name.clone()),
                })
            }
        } else {
            None
        };

        let patient = Patient {
            id: dicom_metadata
                .patient_id
                .as_ref()
                .or(config.patient.id.as_ref())
                .cloned()
                .unwrap_or_else(|| "UNKNOWN".to_string()),
            name: human_name,
            dob: dicom_metadata
                .patient_dob
                .as_ref()
                .or(config.patient.dob.as_ref())
                .cloned(),
            sex: dicom_metadata
                .patient_sex
                .as_ref()
                .or(config.patient.sex.as_ref())
                .cloned(),
            identifiers: config.patient.identifiers.as_ref().map(|ids| {
                ids.iter()
                    .map(|id_config| Identifier {
                        system: id_config.system.clone(),
                        value: id_config.value.clone(),
                    })
                    .collect()
            }),
            verification: config.patient.verification.as_ref().map(|v| Verification {
                verified_by: Some(v.method.clone()),
                verified_on: v.timestamp.as_ref().map(|ts| ts.split('T').next().unwrap_or(ts).to_string()),
            }),
        };

        // Create studies information from DICOM data
        let studies = if !dicom_metadata.series.is_empty() || dicom_metadata.study_description.is_some() {
            Some(Studies {
                study_description: dicom_metadata.study_description.clone(),
                study_uid: dicom_metadata.study_uid.clone(),
                series: Some(dicom_metadata.series.clone()),
            })
        } else {
            None
        };

        // Create report reference
        let report = config.report.as_ref().map(|r| Report {
            file: r.file.clone(),
        });

        // Create extensions (same as manifest)
        let extensions = if config.custom_tags.is_some() || config.consent.is_some() || config.deid_keys.is_some() {
            Some(Extensions {
                custom_tags: config.custom_tags.clone(),
                consent: config.consent.as_ref().map(|c| ConsentExtension {
                    status: c.status.clone(),
                    scope: c.scope.clone(),
                    method: c.method.clone(),
                    signed_on: Some(timestamp.split('T').next().unwrap_or(timestamp).to_string()),
                }),
                deid: config.deid_keys.as_ref().map(|keys| DeidExtension {
                    keys: keys.clone(),
                }),
                additional: HashMap::new(),
            })
        } else {
            None
        };

        Ok(Metadata {
            version: config.version.clone().unwrap_or_else(|| "1.0".to_string()),
            id: envelope_id.to_string(),
            timestamp: timestamp.to_string(),
            patient,
            report,
            studies,
            extensions,
        })
    }

    /// Create audit trail
    fn create_audit(
        &self,
        config: &Config,
        _envelope_id: &str,
        timestamp: &str,
    ) -> JmixResult<Audit> {
        let mut audit_entries = Vec::new();

        // Creation event
        audit_entries.push(AuditEntry {
            event: "created".to_string(),
            timestamp: timestamp.to_string(),
            by: EntityRef {
                id: config.sender.id.clone(),
                name: Some(config.sender.name.clone()),
            },
            to: None,
            assertion: None,
        });

        // Transmission event (if there are receivers)
        if !config.receivers.is_empty() {
            for receiver in &config.receivers {
                audit_entries.push(AuditEntry {
                    event: "transmitted".to_string(),
                    timestamp: timestamp.to_string(),
                    by: EntityRef {
                        id: config.requester.id.clone(),
                        name: Some(config.requester.name.clone()),
                    },
                    to: Some(EntityRef {
                        id: receiver.id.clone(),
                        name: Some(receiver.name.clone()),
                    }),
                    assertion: None,
                });
            }
        }

        Ok(Audit {
            audit: audit_entries,
        })
    }

    /// Collect DICOM files from the source directory
    fn collect_dicom_files<P: AsRef<Path>>(&self, dicom_path: P) -> JmixResult<Vec<PathBuf>> {
        let _processor = DicomProcessor::new();
        // Reuse the processor's logic for finding DICOM files
        let walker = WalkDir::new(dicom_path).into_iter();
        let mut dicom_files = Vec::new();
        
        for entry in walker.filter_map(|e| e.ok()) {
            if entry.file_type().is_file() {
                let file_path = entry.path();
                // Check if it's a DICOM file (reuse processor logic)
                if self.is_dicom_file(file_path)? {
                    dicom_files.push(file_path.to_path_buf());
                }
            }
        }
        
        Ok(dicom_files)
    }
    
    /// Calculate SHA256 hash of a file
    fn sha256_file<P: AsRef<Path>>(&self, file_path: P) -> JmixResult<String> {
        let mut file = fs::File::open(&file_path)?;
        let mut hasher = Sha256::new();
        io::copy(&mut file, &mut hasher)?;
        let hash = hasher.finalize();
        Ok(format!("sha256:{:x}", hash))
    }

    /// Calculate SHA256 hash of in-memory bytes
    fn sha256_bytes(&self, data: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(data);
        let hash = hasher.finalize();
        format!("sha256:{:x}", hash)
    }

    /// Compute a deterministic payload hash over the payload directory.
    /// Algorithm:
    /// - Recursively list files under `payload_dir`
    /// - Sort by relative path (Unicode codepoint order)
    /// - For each file: update hasher with the UTF-8 path, then a newline byte, then the file bytes
    /// - Return sha256:<hex>
    fn compute_payload_hash_for_dir<P: AsRef<Path>>(&self, payload_dir: P) -> JmixResult<String> {
        let payload_dir = payload_dir.as_ref();
        let mut paths: Vec<std::path::PathBuf> = Vec::new();

        // Collect files recursively
        for entry in WalkDir::new(payload_dir).into_iter().filter_map(|e| e.ok()) {
            if entry.file_type().is_file() {
                paths.push(entry.path().to_path_buf());
            }
        }

        // Sort by path relative to payload_dir
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
            let mut file = fs::File::open(&abs_path)?;
            io::copy(&mut file, &mut hasher)?;
        }
        let hash = hasher.finalize();
        Ok(format!("sha256:{:x}", hash))
    }
    
    /// Check if a file is a DICOM file (simplified version of processor logic)
    fn is_dicom_file<P: AsRef<Path>>(&self, file_path: P) -> JmixResult<bool> {
        use dicom_object::open_file;
        
        // First try to parse with dicom-rs
        if let Ok(_) = open_file(&file_path) {
            return Ok(true);
        }

        // Fallback: check for DICOM magic number at offset 128
        let file_path = file_path.as_ref();
        if let Ok(bytes) = fs::read(file_path) {
            if bytes.len() >= 132 && &bytes[128..132] == b"DICM" {
                return Ok(true);
            }
        }

        // Check common DICOM file extensions
        if let Some(extension) = file_path.extension().and_then(|e| e.to_str()) {
            match extension.to_lowercase().as_str() {
                "dcm" | "dicom" | "ima" | "img" => return Ok(true),
                _ => {}
            }
        }

        Ok(false)
    }

    /// Create files manifest from envelope and DICOM files
    fn create_files_manifest(&self, envelope: &Envelope, dicom_files: &[PathBuf]) -> JmixResult<Files> {
        let mut files = Vec::new();

        // Add metadata file entry (in payload/)
        // Hash metadata.json later after writing; for now set None and let caller update if desired
        files.push(FileEntry {
            file: "metadata.json".to_string(),
            hash: None,
            size_bytes: None,
        });

        // Add DICOM files
        for dicom_file in dicom_files {
            if let Some(file_name) = dicom_file.file_name() {
                let dicom_rel = format!("dicom/{}", file_name.to_string_lossy());
                let hash = self.sha256_file(dicom_file).ok();
                let size_bytes = fs::metadata(dicom_file).ok().map(|m| m.len() as i64);
                files.push(FileEntry {
                    file: dicom_rel,
                    hash,
                    size_bytes,
                });
            }
        }

        // Add report file if present (would be in files/ subdirectory)
        if let Some(report) = &envelope.metadata.report {
            if let Some(report_file) = &report.file {
                // Strip the 'files/' prefix since files.json is relative to payload/
                let file_path = if report_file.starts_with("files/") {
                    report_file.clone()
                } else {
                    format!("files/{}", report_file.trim_start_matches("files/"))
                };
                
                let _report_abs = std::path::Path::new(&file_path);
                let hash = None; // unknown until we copy it; currently we don't copy report
                files.push(FileEntry {
                    file: file_path,
                    hash,
                    size_bytes: None,
                });
            }
        }

        Ok(files)
    }

    /// Convert config contact to types contact
    fn convert_contact(&self, contact: &crate::config::ContactInfo) -> Contact {
        match contact {
            crate::config::ContactInfo::Email(email) => Contact::Email(email.clone()),
            crate::config::ContactInfo::Detailed { system, value } => {
                Contact::Point(ContactPoint {
                    system: system.clone(),
                    value: value.clone(),
                })
            }
        }
    }
}

impl Default for JmixBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config as BuildConfig, Entity as CEntity, ContactInfo};
    use tempfile::TempDir;

    #[test]
    fn test_receivers_get_assertions_when_enabled() {
        // Prepare a minimal config with one receiver
        let cfg = BuildConfig {
            version: Some("1.0".to_string()),
            sender: CEntity { name: "Sender Org".into(), id: "org:sender".into(), contact: ContactInfo::Email("sender@example.org".into()), assertion: None },
            requester: CEntity { name: "Requester".into(), id: "org:req".into(), contact: ContactInfo::Email("req@example.org".into()), assertion: None },
            receivers: vec![CEntity { name: "Receiver One".into(), id: "org:recv1".into(), contact: ContactInfo::Email("recv1@example.org".into()), assertion: None }],
            ..Default::default()
        };
        // Create a temp DICOM dir with fake DICM content
        let tmp = TempDir::new().expect("tempdir");
        let dicom_dir = tmp.path().join("dicom");
        std::fs::create_dir_all(&dicom_dir).unwrap();
        std::fs::write(dicom_dir.join("img.dcm"), b"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxDICM").unwrap();

        let builder = JmixBuilder::with_assertions().expect("builder");
        let (envelope, _files) = builder.build_from_dicom(&dicom_dir, &cfg).expect("build");
        assert!(!envelope.manifest.receiver.is_empty());
        assert!(envelope.manifest.receiver[0].assertion.is_some(), "receiver[0] should have an assertion");
    }

    #[test]
    fn test_builder_creation() {
        let builder = JmixBuilder::new();
        assert!(builder.validator.is_none());
        assert!(builder.jws_manager.is_none());
        assert!(builder.encryption_manager.is_none());
        assert!(builder.assertion_manager.is_none());
    }

    #[test]
    fn test_contact_conversion() {
        let builder = JmixBuilder::new();
        
        // Test email contact
        let email_contact = crate::config::ContactInfo::Email("test@example.com".to_string());
        let converted = builder.convert_contact(&email_contact);
        match converted {
            Contact::Email(email) => assert_eq!(email, "test@example.com"),
            _ => panic!("Expected email contact"),
        }

        // Test detailed contact
        let detailed_contact = crate::config::ContactInfo::Detailed {
            system: "phone".to_string(),
            value: "+1234567890".to_string(),
        };
        let converted = builder.convert_contact(&detailed_contact);
        match converted {
            Contact::Point(point) => {
                assert_eq!(point.system, "phone");
                assert_eq!(point.value, "+1234567890");
            }
            _ => panic!("Expected point contact"),
        }
    }

    #[test] 
    fn test_save_to_files() {
        let builder = JmixBuilder::new();
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        
        // Create a minimal envelope for testing
        let envelope = create_test_envelope();
        
        let result = builder.save_to_files(&envelope, &[], temp_dir.path());
        assert!(result.is_ok());
        
        let saved_files = result.unwrap();
        assert_eq!(saved_files.len(), 5); // manifest, audit, metadata, files, README
        
        // Verify files were created
        for file_path in &saved_files {
            assert!(file_path.exists(), "File should exist: {:?}", file_path);
        }
        
        // Verify file names
        let file_names: Vec<String> = saved_files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        
        assert!(file_names.contains(&"manifest.json".to_string()));
        assert!(file_names.contains(&"metadata.json".to_string()));
        assert!(file_names.contains(&"audit.json".to_string()));
        assert!(file_names.contains(&"files.json".to_string()));
        assert!(file_names.contains(&"README.md".to_string()));
    }
    
    #[test] 
    fn test_builder_with_signing() {
        let builder = JmixBuilder::with_signing().unwrap();
        assert!(builder.validator.is_none());
        assert!(builder.jws_manager.is_some());
        assert!(builder.encryption_manager.is_none());
        assert!(builder.assertion_manager.is_none());
        
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let envelope = create_test_envelope();
        
        let result = builder.save_to_files(&envelope, &[], temp_dir.path());
        assert!(result.is_ok());
        
        let saved_files = result.unwrap();
        let file_names: Vec<String> = saved_files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        
        // Should include manifest.jws when signing is enabled
        assert!(file_names.contains(&"manifest.jws".to_string()));
        assert!(file_names.contains(&"manifest.json".to_string()));
    }

    fn create_test_envelope() -> Envelope {
        // Create minimal test envelope
        
        let manifest = Manifest {
            version: "1.0".to_string(),
            id: "test-id".to_string(),
            timestamp: "2024-01-01T00:00:00Z".to_string(),
            sender: Entity {
                name: Some("Test Sender".to_string()),
                id: "test-sender".to_string(),
                contact: Contact::Email("sender@test.com".to_string()),
                assertion: None,
            },
            requester: None,
            receiver: vec![],
            security: Security {
                classification: None,
                payload_hash: "sha256:test".to_string(),
                jws: None,
                signature: None,
                encryption: None,
            },
            extensions: None,
        };

        let metadata = Metadata {
            version: "1.0".to_string(),
            id: "test-id".to_string(),
            timestamp: "2024-01-01T00:00:00Z".to_string(),
            patient: Patient {
                id: "test-patient".to_string(),
                name: None,
                dob: None,
                sex: None,
                identifiers: None,
                verification: None,
            },
            report: None,
            studies: None,
            extensions: None,
        };

        let audit = Audit {
            audit: vec![],
        };

        Envelope {
            manifest,
            metadata,
            audit,
            manifest_jws: None,
        }
    }
}