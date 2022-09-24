use self::binary_search::*;
use anyhow::{anyhow, Error};
use bytes::{Buf, BytesMut};
use std::net::SocketAddr;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task,
};
use tracing::{debug, info};

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

#[derive(Clone, Copy, Debug)]
enum MessageType {
    Insert,
    Query,
}

impl MessageType {
    fn from_byte(byte: u8) -> Result<Self, Error> {
        let s = match byte {
            b'I' => Self::Insert,
            b'Q' => Self::Query,
            _ => return Err(anyhow!("unexpected message type")),
        };
        Ok(s)
    }
}

#[derive(Clone, Copy, Debug)]
struct Message {
    type_: MessageType,
    first: i32,
    second: i32,
}

// message length in bytes
const MESSAGE_LENGTH: usize = 9;

impl Message {
    fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() < MESSAGE_LENGTH {
            return Err(anyhow!("message is {} bytes long", MESSAGE_LENGTH));
        }

        let type_ = MessageType::from_byte(bytes[0])?;

        let first = {
            let mut bs = [0; 4];
            bs.copy_from_slice(&bytes[1..5]);
            i32::from_be_bytes(bs)
        };

        let second = {
            let mut bs = [0; 4];
            bs.copy_from_slice(&bytes[5..9]);
            i32::from_be_bytes(bs)
        };

        let s = Self {
            type_,
            first,
            second,
        };

        Ok(s)
    }
}

#[derive(Debug)]
struct PriceEntry {
    timestamp: i32,
    price: i32,
}

async fn handle_client(mut stream: TcpStream, addr: SocketAddr) -> Result<(), Error> {
    let peer_addr = addr;

    let mut entries = Vec::<PriceEntry>::new();

    let mut read_buf = BytesMut::new();

    loop {
        debug!("{} bytes in read buffer", read_buf.len());

        // fill buffer if it contains less than message length
        if read_buf.len() < MESSAGE_LENGTH {
            let size = stream.read_buf(&mut read_buf).await?;

            debug!("read {} bytes from stream", size);

            debug_assert!(read_buf.len() > 0);
            if size == 0 {
                info!(?peer_addr, "eof");
                break;
            }

            continue;
        }

        // at this point buffer has at least messange length bytes

        // take message length bytes from the buffer
        let bytes = read_buf.copy_to_bytes(9);

        // parse message
        match Message::from_bytes(&*bytes) {
            // message is an insert
            Ok(Message {
                type_: MessageType::Insert,
                first: timestamp,
                second: price,
            }) => {
                // find insert position
                let index = find_insert_index(
                    &entries.iter().map(|x| x.timestamp).collect::<Vec<_>>(),
                    &timestamp,
                );

                let entry = PriceEntry { timestamp, price };

                info!(?peer_addr, "inserting {:?}", entry);

                entries.insert(index, entry);
            }

            // message is a query
            Ok(
                query @ Message {
                    type_: MessageType::Query,
                    first: mintime,
                    second: maxtime,
                },
            ) => {
                info!(?peer_addr, ?query, "received query");

                let avg: i32 = if let Some((min, max)) = find_bounds_index(
                    &entries.iter().map(|x| x.timestamp).collect::<Vec<_>>(),
                    &mintime,
                    &maxtime,
                ) {
                    let entries = &entries[min..=max];
                    if entries.len() == 0 {
                        0
                    } else {
                        // storing sum in i32 overflows, thankfully i64 is big enough for this
                        // problem.
                        //
                        // side note, this issue reminds me of this article https://devblogs.microsoft.com/oldnewthing/20220207-00/?p=106223
                        let sum: i64 = entries.iter().map(|x| x.price as i64).sum();
                        let count = entries.len() as i64;

                        info!("sum {}, count {}", sum, count);

                        (sum / count) as i32
                    }
                } else {
                    0
                };

                info!(?peer_addr, "query result is {}", avg);

                stream.write_all(&avg.to_be_bytes()).await?;
            }

            // fail to parse message
            Err(_) => break,
        }
    }

    info!(?peer_addr, "client disconnected",);

    Ok(())
}

pub async fn run() -> Result<(), Error> {
    info!("starting price store server");
    run_server().await?;
    Ok(())
}

/// Binary search functions.
///
/// I didn't know [partition_point] exists.
///
/// [partition_point]: https://doc.rust-lang.org/std/vec/struct.Vec.html#method.partition_point
mod binary_search {
    use std::cmp;

    pub fn find_insert_index<T>(xs: &[T], x: &T) -> usize
    where
        T: Ord,
    {
        if xs.is_empty() {
            return 0;
        }
        let mut lb = 0;
        let mut ub = xs.len();
        while lb < ub {
            let c = (ub - lb) / 2 + lb;
            let m = &xs[c];
            if m <= x {
                lb = c + 1;
            } else {
                ub = c;
            }
        }
        cmp::max(lb, ub)
    }

    #[cfg(test)]
    #[test]
    fn test_find_insert_index() {
        let xs = [];
        assert_eq!(find_insert_index(&xs, &1), 0);

        let xs = [1];
        assert_eq!(find_insert_index(&xs, &0), 0);
        assert_eq!(find_insert_index(&xs, &1), 1);
        assert_eq!(find_insert_index(&xs, &2), 1);

        let xs = [1, 3];
        assert_eq!(find_insert_index(&xs, &0), 0);
        assert_eq!(find_insert_index(&xs, &1), 1);
        assert_eq!(find_insert_index(&xs, &2), 1);
        assert_eq!(find_insert_index(&xs, &3), 2);
        assert_eq!(find_insert_index(&xs, &4), 2);
    }

    pub fn find_bounds_index<T>(xs: &[T], low: &T, high: &T) -> Option<(usize, usize)>
    where
        T: Ord,
    {
        if low > high {
            return None;
        }

        if xs.is_empty() {
            return None;
        }

        if high < xs.first().unwrap() || low > xs.last().unwrap() {
            return None;
        }

        let low_index = {
            let mut lb = 0;
            let mut ub = xs.len() - 1;
            while lb < ub {
                let c = (ub - lb) / 2 + lb;
                let m = &xs[c];
                if m < low {
                    lb = c + 1;
                } else {
                    ub = c;
                }
            }
            cmp::max(lb, ub)
        };

        let high_index = {
            let mut lb = 0;
            let mut ub = xs.len() - 1;
            while lb < ub {
                let c = if (ub - lb) % 2 == 0 {
                    (ub - lb) / 2 + lb
                } else {
                    (ub + 1 - lb) / 2 + lb
                };
                let m = &xs[c];
                if m <= high {
                    lb = c;
                } else {
                    ub = c - 1;
                }
            }
            cmp::min(lb, ub)
        };

        Some((low_index, high_index))
    }

    #[cfg(test)]
    #[test]
    fn test_find_bounds_index() {
        let xs = [1];
        assert_eq!(find_bounds_index(&xs, &1, &3), Some((0, 0)));
        assert_eq!(find_bounds_index(&xs, &1, &1), Some((0, 0)));
        assert_eq!(find_bounds_index(&xs, &0, &0), None);

        let xs = [1, 2, 3];
        assert_eq!(find_bounds_index(&xs, &1, &3), Some((0, 2)));
        assert_eq!(find_bounds_index(&xs, &1, &1), Some((0, 0)));
        assert_eq!(find_bounds_index(&xs, &0, &0), None);

        let xs = [1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 3];
        assert_eq!(find_bounds_index(&xs, &1, &3), Some((0, xs.len() - 1)));
        assert_eq!(find_bounds_index(&xs, &1, &4), Some((0, xs.len() - 1)));

        let xs = [1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 3, 3];
        assert_eq!(find_bounds_index(&xs, &1, &3), Some((0, xs.len() - 1)));
    }
}
