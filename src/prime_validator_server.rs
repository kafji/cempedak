use anyhow::Error;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    task,
};
use tracing::info;

async fn run_server() -> Result<(), Error> {
    let addr = "0.0.0.0:3000";

    info!("listening at {}", addr);
    let listener = TcpListener::bind(addr).await?;

    loop {
        let (stream, peer_addr) = listener.accept().await?;

        info!(?peer_addr, "received connection");

        task::spawn(handle_client(stream, peer_addr));
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
struct Request {
    method: String,
    number: f64,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
struct Response<'a> {
    method: &'a str,
    prime: bool,
}

impl Response<'_> {
    fn new(prime: bool) -> Self {
        Self {
            method: "isPrime",
            prime,
        }
    }
}

fn is_prime(n: u32) -> bool {
    if n < 2 {
        return false;
    }
    let t = (n as f32).sqrt() as u32;
    for i in (2..=t).rev() {
        if n % i == 0 {
            return false;
        }
    }
    true
}

#[cfg(test)]
#[test]
fn test_is_prime() {
    assert_eq!(is_prime(0), false);
    assert_eq!(is_prime(1), false);
    assert_eq!(is_prime(2), true);
    assert_eq!(is_prime(3), true);
    assert_eq!(is_prime(4), false);
    assert_eq!(is_prime(5), true);
    assert_eq!(is_prime(98995349), true);
    assert_eq!(is_prime(89035527), false);
}

fn is_integer(n: f64) -> bool {
    n % 1.0 == 0.0
}

#[cfg(test)]
#[test]
fn test_is_integer() {
    assert_eq!(is_integer(43446289.0), true);
    assert_eq!(is_integer(43446289.5), false);
}

async fn handle_client(mut stream: TcpStream, addr: SocketAddr) -> Result<(), Error> {
    let peer_addr = addr;

    let (read_stream, mut write_stream) = stream.split();

    let reader = BufReader::new(read_stream);
    let mut requests = reader.lines();

    loop {
        let request = requests.next_line().await?;

        match request {
            Some(request) => {
                info!(?peer_addr, ?request, "received request");

                match serde_json::from_str(&request) {
                    Ok(Request { method, number }) if method == "isPrime" => {
                        let response = if is_integer(number) {
                            let n = number as u32;
                            let x = is_prime(n);
                            Response::new(x)
                        } else {
                            Response::new(false)
                        };

                        let mut response = serde_json::to_string(&response)?;
                        response.push('\n');

                        info!(?response, "sending response");

                        write_stream.write_all(&response.as_bytes()).await?;
                    }
                    _ => {
                        info!(?peer_addr, ?request, "request was malformatted");

                        write_stream.write_all(b"{}\n").await?;
                    }
                }
            }
            None => break,
        }
    }

    Ok(())
}

pub async fn run() -> Result<(), Error> {
    info!("starting prime validator server");
    run_server().await?;
    Ok(())
}
