use harmony::config::{Cli};
use harmony::config::config::Config;

#[tokio::main]
async fn main() {
    // Simulate loading Config
    let cli = Cli::new("/path/to/config.toml".to_string());
    let config = Config::from_args(cli);

    // Pass the Config into your application logic
    harmony::run(config).await;
}
