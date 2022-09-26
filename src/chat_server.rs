//! A chat room server.
//!
//! Unlike previous problems, this requires shared state between clients.
//!
//! My solution was to have a shared mutex across client tasks where any of
//! those tasks can mutate the shared memory and send messages to other peers.
//! That solution was working, however it is difficult for me to reason about
//! its execution and debugging it is just not.. fun.
//!
//! The second solution, this one, is to use a message passing communication
//! between tasks with no memory sharing similar to Erlang/Elixir
//! server/process. Here we have 1 task that controls the room data
//! [spawn_room_task], this task will take messages from user tasks
//! [spawn_user_task] and will inform changes in its state to the other user
//! tasks by sending messages to each of them.

use anyhow::{Context, Error};
use bytes::{Buf, BytesMut};
use futures::{stream::FuturesUnordered, TryStreamExt};
use std::{
    collections::HashMap,
    fmt::Debug,
    io,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, Interest},
    net::{TcpListener, TcpStream},
    select,
    sync::{mpsc, oneshot},
    task::{self, JoinHandle},
};
use tracing::{error, info};

async fn run_server() -> Result<(), Error> {
    let addr = "0.0.0.0:3000";

    info!("listening at {}", addr);
    let listener = TcpListener::bind(addr).await?;

    // spawn the room task
    let (room_tx, room_rx) = mpsc::channel(1);
    spawn_room_task(room_rx);

    loop {
        let (stream, peer_addr) = listener.accept().await?;

        // spawn new user task for each new client
        task::spawn(spawn_user_task(stream, peer_addr, room_tx.clone()));
    }
}

pub async fn run() -> Result<(), Error> {
    info!("starting chat server");
    run_server().await?;
    Ok(())
}

#[derive(Debug)]
enum UserJoinedResult {
    Ok { members: Vec<String> },
    Duplicate,
}

#[derive(Debug)]
enum RoomEvent {
    UserJoined {
        name: String,
        user_event_tx: mpsc::Sender<Arc<UserEvent>>,
        result_tx: oneshot::Sender<UserJoinedResult>,
    },
    UserLeft {
        name: String,
    },
    NewChatMessage {
        name: String,
        message: String,
    },
}

fn spawn_room_task(mut event_rx: mpsc::Receiver<RoomEvent>) -> JoinHandle<()> {
    task::spawn(async move {
        let mut users: HashMap<_, mpsc::Sender<Arc<UserEvent>>> = HashMap::new();

        while let Some(event) = event_rx.recv().await {
            match event {
                RoomEvent::UserJoined {
                    name,
                    user_event_tx,
                    result_tx,
                } => {
                    if users.contains_key(&name) {
                        result_tx.send(UserJoinedResult::Duplicate).unwrap();
                        continue;
                    }

                    let event = Arc::new(UserEvent::UserJoined { name: name.clone() });

                    users
                        .values()
                        .map(|tx| tx.send(event.clone()))
                        .collect::<FuturesUnordered<_>>()
                        .try_collect::<Vec<_>>()
                        .await
                        .unwrap();

                    let names = users.keys().cloned().collect::<Vec<_>>();
                    result_tx
                        .send(UserJoinedResult::Ok { members: names })
                        .unwrap();

                    users.insert(name, user_event_tx);
                }

                RoomEvent::UserLeft { name } => {
                    users.remove(&name);

                    let event = Arc::new(UserEvent::UserLeft { name });

                    users
                        .values()
                        .map(|tx| tx.send(event.clone()))
                        .collect::<FuturesUnordered<_>>()
                        .try_collect::<Vec<_>>()
                        .await
                        .unwrap();
                }

                RoomEvent::NewChatMessage { name, message } => {
                    let event = Arc::new(UserEvent::NewChatMessage {
                        name: name.clone(),
                        message,
                    });

                    users
                        .iter()
                        .filter(|(k, _)| *k != &name)
                        .map(|(_, v)| v)
                        .map(|tx| tx.send(event.clone()))
                        .collect::<FuturesUnordered<_>>()
                        .try_collect::<Vec<_>>()
                        .await
                        .unwrap();
                }
            }
        }
    })
}

#[derive(Debug)]
enum UserEvent {
    NewChatMessage { name: String, message: String },
    UserJoined { name: String },
    UserLeft { name: String },
}

async fn spawn_user_task(
    mut stream: TcpStream,
    addr: SocketAddr,
    room_tx: mpsc::Sender<RoomEvent>,
) {
    // shared memory for name between the main task and its cleanup part
    let cleanup_name: Arc<Mutex<Option<String>>> = Default::default();

    let main = {
        let room_tx = room_tx.clone();
        let cleanup_name = cleanup_name.clone();

        task::spawn(async move {
            let (mut read, mut write) = stream.split();

            let mut outbox = Outbox {
                peer_addr: addr,
                write: &mut write,
                name: None,
            };

            let mut read_buf = Default::default();
            let mut msg_buf = Default::default();
            let mut inbox = Inbox {
                peer_addr: addr,
                read_buf: &mut read_buf,
                read: &mut read,
                msg_buf: &mut msg_buf,
                name: None,
            };

            outbox
                .send_message("Welcome to budgetchat! What shall I call you?")
                .await
                .expect("failed to send name request message");

            let name = inbox
                .recv_message()
                .await
                .expect("failed to read user name message");

            if !is_valid_name(&name) {
                info!(?name, "invalid name");

                outbox.send_message("invalid name").await.ok();

                return ();
            }

            let (user_event_tx, mut user_event_rx) = mpsc::channel(1);

            let (join_result_tx, join_result_rx) = oneshot::channel();

            room_tx
                .send(RoomEvent::UserJoined {
                    name: name.clone(),
                    user_event_tx,
                    result_tx: join_result_tx,
                })
                .await
                .expect("failed to send user joined room event");

            match join_result_rx.await.expect("failed to receive member list") {
                UserJoinedResult::Ok { members } => {
                    let msg = members.iter().enumerate().fold(
                        String::from("* The room contains: "),
                        |mut acc, (i, x)| {
                            if i == 0 {
                                acc.push_str(x);
                            } else {
                                acc.push_str(", ");
                                acc.push_str(x);
                            }
                            acc
                        },
                    );
                    outbox
                        .send_message(&msg)
                        .await
                        .expect("failed to send member list message");

                    outbox.set_name(&name);
                    inbox.set_name(&name);

                    let mut cleanup_name = cleanup_name.lock().unwrap();
                    cleanup_name.replace(name.clone());
                }

                UserJoinedResult::Duplicate => {
                    info!(?name, "duplicate name");

                    outbox.send_message("duplicate name").await.ok();

                    return ();
                }
            }

            loop {
                select! { biased;

                    // listen for message from client
                    Ok(msg) = inbox.recv_message() => {
                        room_tx
                            .send(RoomEvent::NewChatMessage { name: name.clone(), message: msg })
                            .await
                            .expect("failed to send new chat message room event");
                    }

                    // 'poll' for disconnection status
                    // the sleep provides window for receiving message from the room task (next select! arm)
                    _ = tokio::time::sleep(Duration::from_millis(20)) => {
                        let readiness = write.ready(Interest::READABLE.add(Interest::WRITABLE))
                            .await
                            .expect("failed to get readiness");
                        if readiness.is_read_closed() || readiness.is_write_closed() {
                            info!(?name, "connection closed");
                            break;
                        }
                    }

                    // listen for message from the room task
                    Some(event) = user_event_rx.recv() => {
                        let mut outbox = Outbox {
                            peer_addr: addr,
                            write: &mut write,
                            name: Some(&name),
                        };

                        match &*event {
                            UserEvent::NewChatMessage { name, message } => {
                                let msg = format!("[{}] {}", name, message);
                                outbox.send_message(&msg).await.expect("failed to send new chat message message")
                            },
                            UserEvent::UserJoined { name } => {
                                let msg = format!("* {} has entered the room", name);
                                outbox.send_message(&msg).await.expect("failed to send user joined message")
                            },
                            UserEvent::UserLeft { name } => {
                                let msg = format!("* {} has left the room", name);
                                outbox.send_message(&msg).await.expect("failed to send user left message")
                            },
                        }
                    }
                }
            }
        })
    };

    // wait for the main task to complete
    let main_result = main.await;

    // the completion of main task denotes that the client has disconnected

    // do cleanup

    let name = Arc::try_unwrap(cleanup_name).unwrap().into_inner().unwrap();

    info!(?name, "user disconnected");

    if let Some(name) = &name {
        room_tx
            .send(RoomEvent::UserLeft { name: name.clone() })
            .await
            .expect("failed to send user left room event");
    }

    if let Err(err) = main_result {
        error!(?name, ?err);
    }
}

pub struct Outbox<'a, W> {
    pub peer_addr: SocketAddr,
    pub write: &'a mut W,
    pub name: Option<&'a str>,
}

impl<'a, W> Outbox<'a, W>
where
    W: AsyncWrite + Unpin,
{
    pub fn set_name(&mut self, name: &'a str) {
        self.name.replace(name);
    }

    pub async fn send_message(&mut self, msg: &str) -> Result<(), Error> {
        let mut buf = msg.to_owned();
        buf.push('\n');

        info!(name = ?self.name, peer_addr = ?self.peer_addr, msg = ?buf, "sending message");

        self.write
            .write_all(buf.as_bytes())
            .await
            .with_context(|| format!("failed to send bytes to user {:?}", self.name))?;

        Ok(())
    }
}

pub struct Inbox<'a, R> {
    pub peer_addr: SocketAddr,
    pub read_buf: &'a mut BytesMut,
    pub read: &'a mut R,
    pub msg_buf: &'a mut String,
    pub name: Option<&'a str>,
}

impl<'a, R> Inbox<'a, R>
where
    R: AsyncRead + Unpin,
{
    pub fn set_name(&mut self, name: &'a str) {
        self.name.replace(name);
    }

    /// Cancel-safe async IO bytestream read for newline-terminated message.
    pub async fn recv_message(&mut self) -> Result<String, Error> {
        let msg = loop {
            let read = self
                .read
                .read_buf(&mut self.read_buf)
                .await
                .with_context(|| format!("failed to read bytes from user {:?}", self.name))?;

            if read == 0 {
                return Err(Error::from(io::Error::from(io::ErrorKind::UnexpectedEof)))
                    .with_context(|| format!("eof for user {:?}", self.name));
            }

            let msg = loop {
                if self.read_buf.remaining() == 0 {
                    break None;
                }

                let byte = self.read_buf.get_u8();

                if byte == 0x0a {
                    let msg = self.msg_buf.trim().to_owned();
                    self.msg_buf.clear();
                    break Some(msg);
                } else {
                    self.msg_buf.push(byte as _);
                }
            };

            if let Some(msg) = msg {
                break msg;
            }
        };

        info!(name = ?self.name, peer_addr = ?self.peer_addr, ?msg, "message received");

        Ok(msg)
    }
}

pub fn is_valid_name(name: &str) -> bool {
    !name.is_empty() && is_alphanumeric(name)
}

pub fn is_alphanumeric(s: &str) -> bool {
    for c in s.chars() {
        match c {
            'a'..='z' => continue,
            'A'..='Z' => continue,
            '0'..='9' => continue,
            _ => return false,
        }
    }
    true
}
