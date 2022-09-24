use anyhow::{Context, Error};
use std::net::SocketAddr;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
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

async fn handle_client(mut stream: TcpStream, addr: SocketAddr) -> Result<(), Error> {
    let peer_addr = addr;

    loop {
        // assume we will never read data bigger than 1 kb
        let mut buf = [0; 1024];

        let size = stream.read(&mut buf).await.context("failed to read data")?;

        info!(?peer_addr, size, "received data");

        debug_assert!(buf.len() > 0);
        if size == 0 {
            info!(?peer_addr, "eof");
            break;
        }

        // write the received data back
        stream
            .write_all(&buf[..size])
            .await
            .context("failed to write data back")?;
    }

    info!(?peer_addr, "connection terminated");

    Ok(())
}

pub async fn run() -> Result<(), Error> {
    info!("starting echo server");
    run_server().await?;
    Ok(())
}
