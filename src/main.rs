use harmony::config::config::Config;
use harmony::config::env_substitution::{substitute_env_vars, validate_substituted_values};
use harmony::config::Cli;
use std::env;

struct CliArgs {
    config_path: String,
    validate_only: bool,
}

fn parse_cli_args() -> CliArgs {
    let mut args = env::args().skip(1);
    let mut config_path: Option<String> = None;
    let mut validate_only = false;

    while let Some(arg) = args.next() {
        if arg == "--validate-config" {
            validate_only = true;
        } else if arg == "--config" || arg == "-c" {
            if let Some(val) = args.next() {
                config_path = Some(val);
            }
        } else if let Some(val) = arg.strip_prefix("--config=") {
            config_path = Some(val.to_string());
        }
    }

    CliArgs {
        config_path: config_path.unwrap_or_else(|| "./config/config.toml".to_string()),
        validate_only,
    }
}

#[tokio::main]
async fn main() {
    // Parse CLI args
    let args = parse_cli_args();

    if args.validate_only {
        // Validation-only mode: check required vars, perform substitution, parse TOML, and report
        let contents = std::fs::read_to_string(&args.config_path)
            .expect("Failed to read config file");

        // Extract required_env_vars from [proxy]
        let mut missing_required: Vec<String> = Vec::new();
        if let Ok(table) = toml::from_str::<toml::Table>(&contents) {
            if let Some(proxy) = table.get("proxy") {
                if let Some(arr) = proxy.get("required_env_vars").and_then(|v| v.as_array()) {
                    for v in arr {
                        if let Some(s) = v.as_str() {
                            if env::var(s).is_err() {
                                missing_required.push(s.to_string());
                            }
                        }
                    }
                }
            }
        }

        if !missing_required.is_empty() {
            eprintln!(
                "✗ Configuration validation failed\n✗ Missing required environment variables: {}",
                missing_required.join(", ")
            );
            std::process::exit(1);
        }

        // Perform substitution and value validation (names only in logs)
        let (_substituted, audit) = substitute_env_vars(&contents);
        let warnings = validate_substituted_values(&audit);
        for w in warnings.warnings {
            eprintln!("⚠ {}", w);
        }

        // Full load/validate using standard path
        let cli = if args.validate_only {
            Cli::new_validate_only(args.config_path.clone())
        } else {
            Cli::new(args.config_path.clone())
        };
        let _config = Config::from_args(cli);
        println!("✓ Configuration is valid\n✓ All required environment variables present\n✓ Substitution completed");
        std::process::exit(0);
    }

    // Normal startup
    let cli = Cli::new(args.config_path.clone());
    let config = Config::from_args(cli);
    harmony::run_with_reload(config, Some(args.config_path)).await;
}
