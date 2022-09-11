use protohackers::*;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_thread_ids(true)
        .init();

    let problem: u8 = std::env::args()
        .skip(1)
        .next()
        .expect("missing argument for problem number")
        .parse()
        .expect("expecting number for problem number argument");

    match problem {
        0 => echo_server::run().await.unwrap(),
        1 => is_prime_server::run().await.unwrap(),
        2 => price_store_server::run().await.unwrap(),
        _ => panic!("unknown problem"),
    }
}
