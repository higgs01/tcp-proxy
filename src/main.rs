#![warn(rust_2018_idioms)]

use tokio::io;
use tokio::net::{TcpListener, TcpStream};

use futures::future::try_join;
use futures::FutureExt;
use std::env;
use std::error::Error;
use ratelimit_meter::KeyedRateLimiter;
use std::time::Duration;
use std::num::NonZeroU32;
use std::net::Shutdown;
use tokio::prelude::io::AsyncWriteExt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let listen_addr = env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:8081".to_string());
    let server_addr = env::args()
        .nth(2)
        .unwrap_or_else(|| "127.0.0.1:8082".to_string());

    println!("Listening on: {}", listen_addr);
    println!("Proxying to: {}", server_addr);

    let mut listener = TcpListener::bind(listen_addr).await?;
    let mut limiter = KeyedRateLimiter::<&str>::new(NonZeroU32::new(20).unwrap(), Duration::from_secs(5));

    while let Ok((inbound, _)) = listener.accept().await {
        let remoteIp = Box::leak(inbound.peer_addr().unwrap().ip().to_string().into_boxed_str());
        let limiterResult = limiter.check(remoteIp);
        if limiterResult != Ok(()) {
            println!("ratelimit exceeded for {}", inbound.peer_addr().unwrap());
            continue;
        }
        let transfer = transfer(inbound, server_addr.clone()).map(|r| {
            if let Err(e) = r {
                println!("Failed to transfer; error={}", e);
            }
        });

        tokio::spawn(transfer);
    }

    Ok(())
}

async fn transfer(mut inbound: TcpStream, proxy_addr: String) -> Result<(), Box<dyn Error>> {
    let mut outbound = TcpStream::connect(proxy_addr).await?;
    println!("new connection from {}", inbound.peer_addr().unwrap());

    let (mut ri, mut wi) = inbound.split();
    let (mut ro, mut wo) = outbound.split();

    let client_to_server = io::copy(&mut ri, &mut wo);
    let server_to_client = io::copy(&mut ro, &mut wi);

    try_join(client_to_server, server_to_client).await?;

    println!("connection finished");
    Ok(())
}
