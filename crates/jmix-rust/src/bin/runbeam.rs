use clap::{Parser, Subcommand};
use std::path::PathBuf;
use jmix_rs::{validate_package, ValidationOptions};

#[derive(Parser)]
#[command(name = "runbeam", about = "Runbeam CLI - JMIX utilities")] 
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Validate a JMIX package directory
    Validate {
        /// Path to the JMIX package directory (e.g., ./tmp/<id>.jmix)
        #[arg(value_name = "DIR")]
        dir: PathBuf,

        /// Validate JSON against schemas (requires ../jmix/schemas or --schema-dir)
        #[arg(long)]
        validate_schema: bool,

        /// Schema directory (defaults to ../jmix/schemas if not provided)
        #[arg(long, value_name = "PATH")]
        schema_dir: Option<PathBuf>,

        /// Verify sender/requester/receiver assertions if present
        #[arg(long)]
        verify_assertions: bool,

        /// Recipient secret key path for decrypting encrypted packages
        #[arg(long, value_name = "PATH")]
        key: Option<PathBuf>,

        /// Output report as JSON
        #[arg(long)]
        json: bool,
    },

    /// Decrypt and extract an encrypted JMIX payload
    Decrypt {
        /// Path to the JMIX package directory (e.g., ./tmp/<id>.jmix)
        #[arg(value_name = "DIR")]
        dir: PathBuf,

        /// Recipient secret key path for decrypting payload.enc
        #[arg(long, value_name = "PATH")]
        key: PathBuf,

        /// Output directory to extract the decrypted payload.tar
        #[arg(long, value_name = "PATH")]
        out: PathBuf,
    },
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Validate { dir, validate_schema, schema_dir, verify_assertions, key, json } => {
            let opts = ValidationOptions {
                validate_schema,
                schema_dir: schema_dir.map(|p| p.to_string_lossy().to_string()),
                verify_assertions,
                recipient_secret_key_path: key,
            };

            let report = validate_package(&dir, &opts)?;

            if json {
                println!("{}", serde_json::to_string_pretty(&serde_json::json!({
                    "schema_ok": report.schema_ok,
                    "payload_hash_ok": report.payload_hash_ok,
                    "assertions_ok": report.assertions_ok,
                    "encryption_ok": report.encryption_ok,
                    "errors": report.errors,
                }))?);
            } else {
                println!("Validation report for {}:", dir.display());
                if let Some(v) = report.schema_ok { println!("  Schema: {}", if v { "ok" } else { "FAIL" }); }
                if let Some(v) = report.payload_hash_ok { println!("  Payload hash: {}", if v { "ok" } else { "FAIL" }); }
                if let Some(v) = report.assertions_ok { println!("  Assertions: {}", if v { "ok" } else { "FAIL" }); }
                if let Some(v) = report.encryption_ok { println!("  Decryption: {}", if v { "ok" } else { "FAIL" }); }
                if !report.errors.is_empty() {
                    println!("  Errors:");
                    for e in report.errors { println!("    - {}", e); }
                }
            }
        }
        Commands::Decrypt { dir, key, out } => {
            use jmix_rs::encryption::DecryptionManager;
            use std::fs;
            use std::io::Cursor;

            // Read manifest and encryption info
            let manifest_path = dir.join("manifest.json");
            let manifest_str = fs::read_to_string(&manifest_path)?;
            let manifest: serde_json::Value = serde_json::from_str(&manifest_str)?;
            let enc = manifest
                .get("security")
                .and_then(|s| s.get("encryption"))
                .ok_or_else(|| format!("No encryption info found in {}", manifest_path.display()))?;
            let enc_info: jmix_rs::types::EncryptionInfo = serde_json::from_value(enc.clone())?;

            // Read encrypted payload
            let payload_enc_path = dir.join("payload.enc");
            let ciphertext = fs::read(&payload_enc_path)?;

            // Decrypt
            let dec = DecryptionManager::from_secret_key_file(&key)
                .map_err(|e| jmix_rs::error::JmixError::Other(format!("Failed to create decryption manager: {}", e)))?;
            let plaintext_tar = dec
                .decrypt(&ciphertext, &enc_info)
                .map_err(|e| jmix_rs::error::JmixError::Other(format!("Decryption failed: {}", e)))?;

            // Extract
            fs::create_dir_all(&out)?;
            let cursor = Cursor::new(plaintext_tar);
            let mut archive = tar::Archive::new(cursor);
            archive.unpack(&out)?;

            println!(
                "Decrypted payload extracted to {}",
                out.display()
            );
        }
    }

    Ok(())
}
