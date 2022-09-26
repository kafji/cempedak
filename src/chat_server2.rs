//! Alternative of [chat_server].
//!
//! This impl forgone message passing and use mutex exclusively.
//!
//! This impl is deadlock prone.

use anyhow::{bail, Error};
use bytes::BytesMut;
use std::{collections::HashMap, fmt::Debug, net::SocketAddr, sync::Arc, time::Duration};
use tokio::{
    net::{TcpListener, TcpStream},
    select,
    sync::Mutex,
    task,
};
use tracing::info;

use crate::chat_server::{is_valid_name, Inbox, Outbox};

async fn run_server() -> Result<(), Error> {
    let addr = "0.0.0.0:3000";

    info!("listening at {}", addr);
    let listener = TcpListener::bind(addr).await?;

    let room = Room::default();

    loop {
        let (stream, peer_addr) = listener.accept().await?;

        let room = room.clone();
        task::spawn(async move {
            let user = User::new(room, stream, peer_addr);

            user.hello().await.unwrap();

            loop {
                if let Err(_) = user.listen().await {
                    break;
                }
            }
        });
    }
}

pub async fn run() -> Result<(), Error> {
    info!("starting chat server");
    run_server().await?;
    Ok(())
}

#[derive(Clone, Default, Debug)]
struct Room {
    users: Arc<Mutex<HashMap<String /* name */, User>>>,
}

impl Room {
    async fn join(&self, name: String, user: User) -> Result<(), Error> {
        let mut users = self.users.lock().await;

        if users.contains_key(&name) {
            bail!("duplicate name");
        }

        {
            let msg = format!("* {} has entered the room", name);
            for (_, user) in &*users {
                user.send_message(&msg).await;
            }
        }

        {
            let msg = users.keys().enumerate().fold(
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
            user.send_message(&msg).await;
        }

        users.insert(name, user);

        Ok(())
    }

    async fn leave(&self, name: &str) {
        let mut users = self.users.lock().await;

        users.remove(name);

        let msg = format!("* {} has left the room", name);
        for (_, user) in &*users {
            user.send_message(&msg).await;
        }
    }

    async fn new_message(&self, name: &str, message: &str) {
        let users = self.users.lock().await;

        let msg = format!("[{}] {}", name, message);
        for (_, user) in users.iter().filter(|(x, _)| *x != name) {
            user.send_message(&msg).await;
        }
    }
}

#[derive(Clone, Debug)]
struct User {
    room: Room,
    inner: Arc<Mutex<UserInner>>,
}

#[derive(Debug)]
struct UserInner {
    addr: SocketAddr,
    stream: TcpStream,
    name: Option<String>,
    read_buf: BytesMut,
    msg_buf: String,
}

impl UserInner {
    fn outbox(&mut self) -> Outbox<'_, TcpStream> {
        Outbox {
            peer_addr: self.addr,
            write: &mut self.stream,
            name: self.name.as_deref(),
        }
    }

    fn inbox(&mut self) -> Inbox<'_, TcpStream> {
        Inbox {
            peer_addr: self.addr,
            read_buf: &mut self.read_buf,
            read: &mut self.stream,
            msg_buf: &mut self.msg_buf,
            name: self.name.as_deref(),
        }
    }
}

impl User {
    fn new(room: Room, stream: TcpStream, addr: SocketAddr) -> Self {
        let inner = UserInner {
            addr,
            stream,
            name: Default::default(),
            read_buf: Default::default(),
            msg_buf: Default::default(),
        };
        let inner = Arc::new(Mutex::new(inner));
        Self { room, inner }
    }

    async fn send_message(&self, msg: &str) {
        let mut this = self.inner.lock().await;

        this.outbox().send_message(msg).await.unwrap();
    }

    async fn hello(&self) -> Result<(), Error> {
        let mut this = self.inner.lock().await;

        this.outbox()
            .send_message("Welcome to budgetchat! What shall I call you?")
            .await
            .unwrap();

        let name = this.inbox().recv_message().await.unwrap();

        if !is_valid_name(&name) {
            bail!("invalid name")
        }

        this.name = Some(name.clone());

        drop(this);

        self.room.join(name, self.clone()).await
    }

    async fn listen(&self) -> Result<(), Error> {
        let mut this = self.inner.lock().await;

        let mut inbox = this.inbox();
        select! { biased;
            res = inbox.recv_message() => {
                    let name = this.name.as_deref().unwrap().to_owned();

                    drop(this);

                    match res {
                        Ok(msg) => {
                            // if new_message panics, we won't send user left message
                            self.room.new_message(&name, &msg).await;
                        },
                        Err(err) => {
                            self.room.leave(&name).await;

                            return Err(err);
                        },
                    }
            }
            _ = tokio::time::sleep(Duration::from_millis(20)) => ()
        }

        Ok(())
    }
}
