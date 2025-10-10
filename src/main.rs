use harmony::config::config::Config;
use harmony::config::Cli;
use std::env;

fn parse_cli_config_path() -> String {
    let mut args = env::args().skip(1);
    let mut config_path: Option<String> = None;

    while let Some(arg) = args.next() {
        if arg == "--config" || arg == "-c" {
            if let Some(val) = args.next() {
                config_path = Some(val);
                break;
            }
        } else if let Some(val) = arg.strip_prefix("--config=") {
            config_path = Some(val.to_string());
            break;
        }
    }

    config_path.unwrap_or_else(|| "examples/config/config.toml".to_string())
}

#[tokio::main]
async fn main() {
    // Parse --config/-c from CLI or fall back to the example config
    let config_path = parse_cli_config_path();
    let cli = Cli::new(config_path);
    let config = Config::from_args(cli);

    // Pass the Config into your application logic
    harmony::run(config).await;
}
