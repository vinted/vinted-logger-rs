use bytes::Bytes;
use futures_channel::mpsc::{self, Receiver};
use futures_util::{SinkExt, StreamExt};
use std::{io, net::SocketAddr};
use tokio::net::{lookup_host, ToSocketAddrs, UdpSocket};
use tokio::time;
use tokio_util::codec::BytesCodec;
use tokio_util::udp::UdpFramed;
use tracing_subscriber::fmt::MakeWriter;

const DEFAULT_BUFFER: usize = 512;
const DEFAULT_TIMEOUT: u32 = 10_000;

#[derive(Debug, Clone)]
pub struct VintedUdpWriter {
    sender: mpsc::Sender<Bytes>,
}

impl VintedUdpWriter {
    /// Returns a new `VintedUdpWriter` with udp configuration
    pub fn new(address: &'static str) -> Self {
        let (sender, receiver) = mpsc::channel::<Bytes>(DEFAULT_BUFFER);

        background_task(address, receiver);

        Self { sender }
    }
}

impl io::Write for VintedUdpWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let _ = self.sender.clone().try_send(Bytes::from(buf.to_owned()));
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter for VintedUdpWriter {
    type Writer = Self;

    fn make_writer(&self) -> Self::Writer {
        (*self).clone()
    }
}

fn background_task<T>(addr: T, mut receiver: Receiver<Bytes>)
where
    T: ToSocketAddrs,
    T: Send + Sync + 'static,
{
    let task = Box::pin(async move {
        // Reconnection loop
        loop {
            // Do a DNS lookup if `addr` is a hostname
            let addrs = lookup_host(&addr).await.into_iter().flatten();

            // Loop through the IP addresses that the hostname resolved to
            for addr in addrs {
                handle_udp_connection(addr, &mut receiver).await;
            }

            // Sleep before re-attempting
            time::sleep(time::Duration::from_millis(DEFAULT_TIMEOUT as u64)).await;
        }
    });

    tokio::spawn(task);
}

async fn handle_udp_connection(addr: SocketAddr, receiver: &mut Receiver<Bytes>) {
    // Bind address version must match address version
    let bind_addr = if addr.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    };
    // Try connect
    let udp_socket = match UdpSocket::bind(bind_addr).await {
        Ok(ok) => ok,
        Err(_) => {
            return;
        }
    };

    // Writer
    let udp_stream = UdpFramed::new(udp_socket, BytesCodec::new());
    let (mut sink, _) = udp_stream.split();
    while let Some(bytes) = receiver.next().await {
        if let Err(_err) = sink.send((bytes, addr)).await {
            // TODO: Add handler
            break;
        };
    }
}
