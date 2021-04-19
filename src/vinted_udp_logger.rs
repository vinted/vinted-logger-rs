#![warn(
    missing_debug_implementations,
    missing_docs,
    unused_must_use,
    unreachable_pub
)]

//! Provides Graylog structured logging using the [`tracing`].
//!
//! # Usage
//!
//! ```rust
//! use std::net::SocketAddr;
//! use tracing_gelf::Logger;
//!
//! #[tokio::main]
//! async fn main() {
//!     // Graylog address
//!     let address: SocketAddr = "127.0.0.1:12201".parse().unwrap();
//!
//!     // Start tracing
//!     let bg_task = Logger::builder().init_tcp(address).unwrap();
//!
//!     // Spawn background task
//!     // Any futures executor can be used
//!     tokio::spawn(bg_task);
//!
//!     // Send a log to Graylog
//!     tracing::info!(message = "oooh, what's in here?");
//!
//!     // Create a span
//!     let span = tracing::info_span!("cave");
//!     span.in_scope(|| {
//!         // Log inside a span
//!         let test = tracing::info_span!("deeper in cave", smell = "damp");
//!         test.in_scope(|| {
//!             tracing::warn!(message = "oh god, it's dark in here");
//!         })
//!     });
//!
//!     // Log a structured log
//!     tracing::error!(message = "i'm glad to be out", spook_lvl = 3, ruck_sack = ?["glasses", "inhaler", "large bat"]);
//!
//! }
//! ```
//!
//! # GELF Encoding
//!
//! [`Events`] are encoded into [`GELF format`](https://docs.graylog.org/en/3.1/pages/gelf.html)
//! as follows:
//! * [`Event`] fields are inserted as [`GELF`] additional fields, `_field_name`.
//! * [`Event`] field named `message` is renamed to `short_message`.
//! * If `short_message` (or `message`) [`Event`] field is missing then `short_message` is
//! set to the empty string.
//! * [`Event`] fields whose names collide with [`GELF`] required fields are coerced
//! into the required types and overrides defaults given in the builder.
//! * The hierarchy of spans is concatenated and inserted as `span_a:span_b:span_c` and
//! inserted as an additional field `_span`.
//!
//! [`tracing`]: https://docs.rs/tracing
//! [`Event`]: https://docs.rs/tracing/0.1.11/tracing/struct.Event.html
//! [`Events`]: https://docs.rs/tracing/0.1.11/tracing/struct.Event.html
//! [`GELF`]: https://docs.graylog.org/en/3.1/pages/gelf.html

pub mod visitor;

use std::future::Future;
use std::net::SocketAddr;

use bytes::Bytes;
use chrono::{DateTime, Utc};
use futures_channel::mpsc;
use futures_util::stream::Stream;
use futures_util::{SinkExt, StreamExt};
use serde_json::{map::Map, Value};
use tokio::net::{lookup_host, ToSocketAddrs, UdpSocket};
use tokio::time;
use tokio_util::codec::BytesCodec;
use tokio_util::udp::UdpFramed;
use tracing_core::dispatcher::SetGlobalDefaultError;
use tracing_core::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::{registry::LookupSpan, Registry};

type BackgroundTask = std::pin::Pin<Box<dyn Future<Output = ()> + Send>>;
const DEFAULT_BUFFER: usize = 512;
const DEFAULT_TIMEOUT: u32 = 10_000;

/// The error type for [`Logger`](struct.Logger.html) building.
#[derive(Debug)]
#[non_exhaustive]
pub enum BuilderError {
    /// Could not resolve the hostname.
    HostnameResolution(std::io::Error),
    /// Could not coerce the OsString into a string.
    OsString(std::ffi::OsString),
    /// Global dispatcher failed.
    Global(SetGlobalDefaultError),
}

/// `Logger` represents a [`Layer`] responsible for sending structured logs to Fluentd.
///
/// [`Layer`]: https://docs.rs/tracing-subscriber/0.2.0-alpha.2/tracing_subscriber/layer/trait.Layer.html
#[derive(Debug)]
pub struct Logger {
    base_object: Map<String, Value>,
    facility: String,
    host: String,
    environment: String,
    sender: mpsc::Sender<Bytes>,
}

/// TODO: explain idea
#[derive(Debug)]
pub struct LogRecord {
    environment: String,
    facility: String,
    host: String,
    target: String,
    module: Option<String>,
    file: Option<String>,
    level: String,
    message: String,
    timestamp: DateTime<Utc>,
    sender: mpsc::Sender<Bytes>,
}

impl Logger {
    /// Create a default [`Logger`] configuration, which can then be customized.
    pub fn builder() -> Builder {
        Builder::default()
    }
}

/// A builder for [`Logger`](struct.Logger.html).
#[derive(Debug)]
pub struct Builder {
    additional_fields: Map<String, Value>,
    facility: String,
    host: String,
    environment: String,
}

impl Default for Builder {
    fn default() -> Self {
        Builder {
            additional_fields: Map::with_capacity(16),
            environment: "dev".into(),
            facility: "new_facility".into(),
            host: "".into(),
        }
    }
}

impl Builder {
    /// Add a persistent additional field to Fluentd messages
    pub fn additional_field<K: ToString, V: Into<Value>>(mut self, key: K, value: V) -> Self {
        let coerced_value: Value = match value.into() {
            Value::Number(n) => Value::Number(n),
            Value::String(x) => Value::String(x),
            x => Value::String(x.to_string()),
        };
        self.additional_fields
            .insert(format!("_{}", key.to_string()), coerced_value);
        self
    }

    /// Set environment
    pub fn environment<V: ToString>(mut self, environment: V) -> Self {
        self.environment = environment.to_string();
        self
    }

    /// Set facility
    pub fn facility<V: ToString>(mut self, facility: V) -> Self {
        self.facility = facility.to_string();
        self
    }

    /// Set host
    pub fn host<V: ToString>(mut self, host: V) -> Self {
        self.host = host.to_string();
        self
    }

    /// Return `Logger` layer and a UDP connection background task.
    pub fn connect_udp<T>(self, addr: T) -> Result<(Logger, BackgroundTask), BuilderError>
    where
        T: ToSocketAddrs,
        T: Send + Sync + 'static,
    {
        // Persistent fields
        let mut base_object = self.additional_fields;

        base_object.insert("host".to_string(), self.host.into());
        base_object.insert("environment".to_string(), self.environment.into());
        base_object.insert("facility".to_string(), self.facility.into());

        // Construct background task
        let (sender, mut receiver) = mpsc::channel::<Bytes>(DEFAULT_BUFFER);

        let bg_task = Box::pin(async move {
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
        let logger = Logger {
            base_object,
            environment: "dev".into(),
            facility: "new_facility".into(),
            host: "".into(),
            sender,
        };

        Ok((logger, bg_task))
    }

    /// Initialize logging with a given `Subscriber` and return UDP connection background task.
    pub fn init_udp_with_subscriber<S, T>(
        self,
        addr: T,
        subscriber: S,
    ) -> Result<BackgroundTask, BuilderError>
    where
        S: Subscriber + for<'a> LookupSpan<'a>,
        S: Send + Sync + 'static,
        T: ToSocketAddrs,
        T: Send + Sync + 'static,
    {
        let (logger, bg_task) = self.connect_udp(addr)?;
        let subscriber = logger.with_subscriber(subscriber);
        tracing_core::dispatcher::set_global_default(tracing_core::dispatcher::Dispatch::new(
            subscriber,
        ))
        .map_err(BuilderError::Global)?;

        Ok(bg_task)
    }

    /// Initialize logging and return UDP connection background task.
    pub fn init_udp<T>(self, addr: T) -> Result<BackgroundTask, BuilderError>
    where
        T: ToSocketAddrs,
        T: Send + Sync + 'static,
    {
        self.init_udp_with_subscriber(addr, Registry::default())
    }
}

impl<S> Layer<S> for Logger
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let mut object: Map<String, Value> = Map::<String, Value>::new();
        let now = Utc::now().to_string();

        // Extract metadata
        // Insert level
        let metadata = event.metadata();
        let level_num = match *metadata.level() {
            tracing_core::Level::ERROR => "Error".to_string(),
            tracing_core::Level::WARN => "Warn".to_string(),
            tracing_core::Level::INFO => "Info".to_string(),
            tracing_core::Level::DEBUG => "Debug".to_string(),
            tracing_core::Level::TRACE => "Trace".to_string(),
        };
        object.insert("level".to_string(), level_num.into());
        object.insert("timestamp".to_string(), now.into());

        // Append additional fields
        let mut add_field_visitor = visitor::AdditionalFieldVisitor::new(&mut object);
        event.record(&mut add_field_visitor);

        // Serialize
        let final_object = Value::Object(object);
        let mut raw = serde_json::to_vec(&final_object).unwrap(); // This is safe
        raw.push(0);

        // Send
        self.sender.clone().try_send(Bytes::from(raw));
    }
}

async fn handle_udp_connection<S>(addr: SocketAddr, receiver: &mut S)
where
    S: Stream<Item = Bytes>,
    S: Unpin,
{
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

/// Creates a Vinted Rust logger
///
/// When environment is production - it'll create fluentd UDP logger,
/// otherwise it'll create console logger
///
/// Usage:
/// ```
/// # #[macro_use]
/// # extern crate tracing;
/// # fn main() {
/// let _ = vinted_logger::from_config("production", "mc-logger");
/// tracing::info!!("Log some stuff");
/// # }
/// ```
pub fn from_config(environment: impl AsRef<str>, facility: impl AsRef<str>) {
    let facility = facility.as_ref();
    let environment = environment.as_ref();

    if environment == "production" {
        let fluentd_task = Logger::builder()
            .facility(facility)
            .environment(environment)
            .init_udp("127.0.0.1:5005")
            .unwrap();

        tokio::spawn(fluentd_task);
    } else {
        // Log to console ?
    }
}
