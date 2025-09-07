use harmony::config::{Cli};
use harmony::config::config::Config;

#[tokio::main]
async fn main() {

    let cli = Cli::new("/path/to/config.toml".to_string()); /// @todo
    let config = Config::from_args(cli);
    harmony::run(config).await;
}