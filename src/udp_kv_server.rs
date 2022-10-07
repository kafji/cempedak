use anyhow::Error;
use std::{
    collections::HashMap,
    net::SocketAddr,
    str::FromStr,
    sync::{Arc, Mutex},
};
use tokio::{net::UdpSocket, task};
use tracing::info;

#[derive(Clone, Default, Debug)]
struct Store {
    kv: Arc<Mutex<HashMap<String, String>>>,
}

impl Store {
    fn put(&self, key: String, value: String) {
        let mut kv = self.kv.lock().unwrap();
        kv.insert(key, value);
    }

    fn get(&self, key: &str) -> String {
        let kv = self.kv.lock().unwrap();
        kv.get(key).cloned().unwrap_or_default()
    }
}

#[derive(PartialEq, Debug)]
enum Request {
    Insert { key: String, value: String },
    Retrieve { key: String },
}

impl FromStr for Request {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // assume only valid ascii
        let s = s.as_bytes();

        let mut key = String::new();

        let mut i = 0;

        while i < s.len() && s[i] != b'=' {
            key.push(s[i] as _);
            i += 1;
        }

        if i >= s.len() {
            return Ok(Self::Retrieve { key });
        }

        let value = s[i + 1..].iter().map(|x| *x as char).collect::<String>();

        return Ok(Self::Insert { key, value });
    }
}

async fn run_server() -> Result<(), Error> {
    let addr = "0.0.0.0:3000";

    info!("listening at {}", addr);

    let socket = UdpSocket::bind(addr).await?;
    let socket = Arc::new(socket);

    let store: Store = Default::default();
    store.put("version".to_owned(), "Ken's Key-Value Store 1.0".to_owned());

    loop {
        let mut read_buf = [0; 2048];

        let (size, peer_addr) = socket.recv_from(&mut read_buf).await?;

        let packet = read_buf[0..size].to_vec();
        task::spawn(handle_packet(
            peer_addr,
            packet,
            store.clone(),
            socket.clone(),
        ));
    }
}

async fn handle_packet(
    peer_addr: SocketAddr,
    packet: Vec<u8>,
    store: Store,
    socket: Arc<UdpSocket>,
) -> Result<(), Error> {
    let msg = String::from_utf8(packet)?;

    info!(?peer_addr, ?msg, "handling packet");

    let request: Request = msg.parse()?;

    match request {
        Request::Insert { key, value } => {
            if key != "version" {
                store.put(key, value);
            }
        }
        Request::Retrieve { key } => {
            let value = store.get(&key);

            let response = format!("{}={}", key, value);

            socket.send_to(response.as_bytes(), peer_addr).await?;
        }
    }

    Ok(())
}

pub async fn run() -> Result<(), Error> {
    info!("starting udp kv server");
    run_server().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_request() {
        assert_eq!(
            "foo=bar".parse::<Request>().unwrap(),
            Request::Insert {
                key: "foo".to_owned(),
                value: "bar".to_owned()
            }
        );

        assert_eq!(
            "foo=".parse::<Request>().unwrap(),
            Request::Insert {
                key: "foo".to_owned(),
                value: "".to_owned()
            }
        );

        assert_eq!(
            "=foo".parse::<Request>().unwrap(),
            Request::Insert {
                key: "".to_owned(),
                value: "foo".to_owned()
            }
        );

        assert_eq!(
            "foo=bar=baz".parse::<Request>().unwrap(),
            Request::Insert {
                key: "foo".to_owned(),
                value: "bar=baz".to_owned()
            }
        );

        assert_eq!(
            "foo===".parse::<Request>().unwrap(),
            Request::Insert {
                key: "foo".to_owned(),
                value: "==".to_owned()
            }
        );

        assert_eq!(
            "foo".parse::<Request>().unwrap(),
            Request::Retrieve {
                key: "foo".to_owned()
            }
        );

        assert_eq!(
            "=".parse::<Request>().unwrap(),
            Request::Insert {
                key: "".to_owned(),
                value: "".to_owned()
            }
        );

        assert_eq!(
            "".parse::<Request>().unwrap(),
            Request::Retrieve { key: "".to_owned() }
        );
    }
}
