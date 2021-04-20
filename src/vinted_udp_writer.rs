use bytes::Bytes;
use std::{
    io,
    net::UdpSocket,
    sync::{
        mpsc::{channel, Sender},
        Mutex,
    },
};
use tracing_subscriber::fmt::MakeWriter;

#[derive(Debug)]
pub(crate) struct VintedUdpWriter {
    addr: &'static str,
    sender: Mutex<Sender<Bytes>>,
}

impl VintedUdpWriter {
    pub(crate) fn new(addr: &'static str) -> Self {
        let (sender, receiver) = channel::<Bytes>();

        let _ = ::std::thread::spawn(move || {
            match UdpSocket::bind("127.0.0.1:0") {
                Ok(socket) => loop {
                    match receiver.recv() {
                        Ok(bytes) => {
                            if let Err(e) = socket.send_to(&bytes, addr) {
                                eprintln!("Log record can't be sent to fluentd: {}", e);
                            }
                        }
                        Err(e) => {
                            eprintln!("Can't receive new log record: {}", e);
                            break;
                        }
                    }
                },
                Err(e) => {
                    eprintln!("Couldn't bind to UDP socket: {}", e);
                }
            };
        });

        Self {
            addr,
            sender: Mutex::new(sender),
        }
    }
}

impl io::Write for VintedUdpWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let _ = self
            .sender
            .lock()
            .unwrap()
            .send(Bytes::from(buf.to_owned()));

        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter for VintedUdpWriter {
    type Writer = Self;

    fn make_writer(&self) -> Self::Writer {
        Self::new(self.addr)
    }
}
