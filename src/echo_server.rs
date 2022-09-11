use anyhow::Error;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use tracing::info;

async fn run_server() -> Result<(), Error> {
    let addr = "0.0.0.0:3000";
    info!("listening at {}", addr);
    let listener = TcpListener::bind(addr).await?;

    loop {
        info!("waiting for new connection");
        let (stream, addr) = listener.accept().await?;
        info!("received connection from {}", addr);
        tokio::task::spawn(handle_client(stream));
    }
}

async fn handle_client(mut stream: TcpStream) -> Result<(), Error> {
    let peer_addr = stream.peer_addr()?;
    loop {
        let mut buf = [0; 1024];
        info!("waiting for data from {}", peer_addr);
        let size = stream.read(&mut buf).await?;
        info!(size, "received data from {}", peer_addr);
        if size == 0 {
            break;
        }
        stream.write_all(&buf[..size]).await?;
    }
    info!("connection with {} was terminated", peer_addr);
    Ok(())
}

pub async fn run() -> Result<(), Error> {
    info!("starting echo server");
    run_server().await?;
    Ok(())
}
