use harmony::config::Config;

#[tokio::main]
async fn main() {
    let config = Config::from_args();
    harmony::run(config).await;
}