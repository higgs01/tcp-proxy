#![warn(rust_2018_idioms)]

use tokio::io;
use tokio::net::{TcpListener, TcpStream};

use futures::future::try_join;
use futures::{FutureExt, TryFutureExt};
use std::env;
use std::error::Error;
use ratelimit_meter::KeyedRateLimiter;
use std::time::Duration;
use std::num::NonZeroU32;
use std::net::Shutdown;
use tokio::prelude::io::{AsyncWriteExt, AsyncReadExt};
use std::io::copy;
use std::borrow::BorrowMut;
use tokio::join;
use std::str;
pub mod protocol;


#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let listen_addr = env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:8081".to_string());
    let server_addr = env::args()
        .nth(2)
        .unwrap_or_else(|| "127.0.0.1:25577".to_string());

    println!("Listening on: {}", listen_addr);
    println!("Proxying to: {}", server_addr);

    let mut listener = TcpListener::bind(listen_addr).await?;
    let mut limiter = KeyedRateLimiter::<&str>::new(NonZeroU32::new(20).unwrap(), Duration::from_secs(5));

    while let Ok((mut inbound, _)) = listener.accept().await {
        let remoteIp = Box::leak(inbound.peer_addr().unwrap().ip().to_string().into_boxed_str());
        let limiterResult = limiter.check(remoteIp);
        if limiterResult != Ok(()) {
            // inbound.write_all(&[0x7b, 0x22, 0x74, 0x65, 0x78, 0x74, 0x22, 0x3a, 0x20, 0x22, 0x66, 0x6f,
            //     0x6f, 0x22, 0x7d]).await;
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
    let port = inbound.peer_addr().unwrap().port();
    println!("new connection from {}", inbound.peer_addr().unwrap());

    let (mut ri, mut wi) = inbound.split();
    let (mut ro, mut wo) = outbound.split();

    // wo.write_all(b"PROXY IPV4 255.255.255.255 255.255.255.255 56412 25577\r\n").await?;
    // outbound.write(b"PROXY IPV4 0.0.0.0 0.0.0.0 51234 25577\r\n");
    // outbound.flush();
    //
    // let client_to_server = io::copy(&mut ri, &mut wo);
    // let server_to_client = io::copy(&mut ro, &mut wi);
    //
    // client_to_server.and_then();
    wo.write_all(&[0x0D, 0x0A, 0x0D, 0x0A, 0x00, 0x0D, 0x0A, 0x51, 0x55, 0x49, 0x54, 0x0A, 0x21, 0x11, 0x00, 0x0C, 0x7F, 0x11, 0x00, 0x01, 0x7F, 0x00, 0x00, 0x01, 0xd3, 0x69, 0x63, 0xe9]).await;

    let client_to_server = async move {
        let mut buffer = Box::new([0; 10000]);
        loop {
            match ri.read(buffer.as_mut()).await {
                Err(_) => break,
                Ok(size) if size == 0 => break,
                Ok(size) => {
                    let mut vec = buffer.to_vec();
                    wo.write_all(&vec[0..size]).await;
                    let s = String::from_utf8_lossy(&vec);
                    println!("packet {}", s)
                }
            };
        }
        // add error handling
        AsyncWriteExt::shutdown(&mut wo).await;
    };

    let server_to_client = async move {
        let mut buffer = Box::new([0; 2000]);
        loop {
            match ro.read(buffer.as_mut()).await {
                Err(_) => break,
                Ok(size) if size == 0 => break,
                Ok(size) => wi.write_all(&buffer[0..size]).await
            };
        }
        // add error handling
        AsyncWriteExt::shutdown(&mut wi).await;
    };

    join!(client_to_server, server_to_client);

    println!("connection closed");
    Ok(())
}
