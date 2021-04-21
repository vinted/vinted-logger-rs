use bytes::Bytes;
use parking_lot::Mutex;
use std::{
    io,
    net::UdpSocket,
    sync::{
        mpsc::{channel, Sender},
        Arc,
    },
};
use tracing_subscriber::fmt::MakeWriter;

pub(crate) struct VintedUdpWriter {
    writer: WriterImpl,
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
            writer: WriterImpl(Arc::new(Mutex::new(sender))),
        }
    }
}

impl MakeWriter for VintedUdpWriter {
    type Writer = WriterImpl;

    fn make_writer(&self) -> Self::Writer {
        self.writer.clone()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct WriterImpl(Arc<Mutex<Sender<Bytes>>>);

impl io::Write for WriterImpl {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let _ = self.0.lock().send(Bytes::from(buf.to_owned()));

        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
