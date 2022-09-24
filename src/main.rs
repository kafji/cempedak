use protohackers::*;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_thread_ids(true)
        .init();

    let problem: Option<u8> = std::env::args()
        .skip(1)
        .next()
        .map(|x| x.parse())
        .transpose()
        .ok()
        .flatten();

    match problem {
        Some(0) => echo_server::run().await.unwrap(),
        Some(1) => prime_validator_server::run().await.unwrap(),
        Some(2) => price_store_server::run().await.unwrap(),
        Some(3) => chat_server::run().await.unwrap(),
        Some(n) => println!("unknown problem, was {}", n),
        None => println!("missing problem argument"),
    }
}
