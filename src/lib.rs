//! Vinted logger for Rust applications

#![deny(
    warnings,
    bad_style,
    const_err,
    dead_code,
    improper_ctypes,
    non_shorthand_field_patterns,
    no_mangle_generic_items,
    overflowing_literals,
    path_statements,
    patterns_in_fns_without_body,
    private_in_public,
    unconditional_recursion,
    unused,
    unused_allocation,
    unused_comparisons,
    unused_parens,
    while_true,
    missing_debug_implementations,
    missing_docs,
    trivial_casts,
    trivial_numeric_casts,
    unused_extern_crates,
    unused_import_braces,
    unused_qualifications,
    unused_results,
    trivial_numeric_casts,
    unreachable_pub,
    unused_extern_crates,
    unused_import_braces,
    unused_qualifications,
    unused_results,
    deprecated,
    unconditional_recursion,
    unknown_lints,
    unreachable_code,
    unused_mut
)]

#[macro_use]
extern crate serde_with;

use chrono::{DateTime, Utc};
use log::{LevelFilter, Record};
use log4rs;
use log4rs::append::console::ConsoleAppender;
use log4rs::config::{Appender, Logger, Root};
use log4rs::encode::json::JsonEncoder;
use log4rs::Config;
use poston::client::{Client, Settings, WorkerPool};
use std::fmt::Debug;
use std::net::ToSocketAddrs;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Mutex;

#[skip_serializing_none]
#[derive(Serialize)]
struct LogRecord {
    facility: String,
    host: String,
    module: Option<String>,
    file: Option<String>,
    level: String,
    message: String,
    #[serde(rename = "@timestamp")]
    timestamp: DateTime<Utc>,
}

impl LogRecord {
    fn new(
        facility: &str,
        host: &str,
        module: Option<&str>,
        file: Option<&str>,
        level: &str,
        message: &str,
    ) -> Self {
        LogRecord {
            facility: facility.into(),
            host: host.into(),
            module: module.map(Into::into),
            file: file.map(Into::into),
            level: level.into(),
            message: message.into(),
            timestamp: Utc::now(),
        }
    }
}

#[derive(Debug)]
struct FluentdAppender {
    encoder: Box<dyn log4rs::encode::Encode>,
    sender: Mutex<Sender<LogRecord>>,
    facility: String,
    host: String,
}

impl FluentdAppender {
    fn builder() -> FluentdAppenderBuilder {
        FluentdAppenderBuilder {
            encoder: None,
            facility: "".into(),
        }
    }
}

impl ::log4rs::append::Append for FluentdAppender {
    fn append(&self, record: &Record) -> Result<(), anyhow::Error> {
        let log_record = LogRecord::new(
            record.target(),
            &self.host,
            record.module_path(),
            record.file(),
            &format!("{}", record.level()),
            &record.args().to_string(),
        );

        let sender = self.sender.lock().unwrap();

        sender.send(log_record)?;

        Ok(())
    }

    fn flush(&self) {}
}

/// `FluentdAppender` builder.
struct FluentdAppenderBuilder {
    encoder: Option<Box<dyn log4rs::encode::Encode>>,
    facility: String,
}

impl FluentdAppenderBuilder {
    /// Set custom encoder.
    fn encoder(mut self, encoder: Box<dyn log4rs::encode::Encode>) -> Self {
        self.encoder = Some(encoder);
        self
    }

    /// Sets facility name
    fn facility(mut self, facility: &str) -> Self {
        self.facility = facility.into();
        self
    }

    /// Consume builder and produce `FluentdAppender`.
    fn build<A>(self, addr: A) -> FluentdAppender
    where
        A: ToSocketAddrs + Clone,
        A: Send + 'static,
        A: Debug,
    {
        let (sender, receiver): (Sender<LogRecord>, Receiver<LogRecord>) =
            ::std::sync::mpsc::channel();
        let facility_clone = self.facility.clone();
        let facility_clone2 = self.facility.clone();

        //Thread receiving all log_record and sending them to fluentd
        let _ = ::std::thread::spawn(move || {
            let settings = Settings {
                connection_retry_timeout: ::std::time::Duration::from_secs(5),
                ..Default::default()
            };

            match WorkerPool::with_settings(&addr, &settings) {
                Ok(pool) => loop {
                    match receiver.recv() {
                        Ok(log_record) => {
                            if let Err(e) = pool.send(
                                facility_clone.clone(),
                                &log_record,
                                log_record.timestamp.into(),
                            ) {
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
                    eprintln!("Fluentd worker pool can't be created: {}", e);
                }
            };
        });

        FluentdAppender {
            encoder: self
                .encoder
                .unwrap_or_else(|| Box::new(log4rs::encode::pattern::PatternEncoder::default())),
            sender: Mutex::new(sender),
            facility: facility_clone2,
            host: gethostname::gethostname()
                .into_string()
                .map(|x| x)
                .unwrap_or("unknown".into()),
        }
    }
}

#[derive(Deserialize)]
struct FluentdAppenderConfig {
    addr: String,
    facility: String,
    encoder: Option<log4rs::encode::EncoderConfig>,
}

struct FluentdAppenderDeserializer;

impl log4rs::config::Deserialize for FluentdAppenderDeserializer {
    type Trait = dyn log4rs::append::Append;
    type Config = FluentdAppenderConfig;

    fn deserialize(
        &self,
        config: Self::Config,
        deserializers: &log4rs::config::Deserializers,
    ) -> anyhow::Result<Box<Self::Trait>> {
        let mut builder = FluentdAppender::builder();

        if let Some(encoder) = config.encoder {
            builder = builder.encoder(deserializers.deserialize(&encoder.kind, encoder.config)?);
        }

        builder = builder.facility(&config.facility);

        Ok(Box::new(builder.build(config.addr)))
    }
}

/// Creates a Vinted Rust logger
///
/// When environment is production - it'll create fluentd logger,
/// otherwise it'll create console logger
///
/// Usage:
/// ```
/// # #[macro_use]
/// # extern crate log;
/// # fn main() {
/// let _ = vinted_logger::from_config("production", "mclogger");
/// info!("Log some stuff");
/// # }
/// ```
pub fn from_config(
    environment: impl AsRef<str>,
    facility: impl AsRef<str>,
) -> Result<log4rs::Handle, Box<dyn std::error::Error + Send + Sync>> {
    let facility = facility.as_ref();

    let config = if environment.as_ref() == "production" {
        let fluentd = FluentdAppender::builder()
            .facility(facility)
            .encoder(Box::new(JsonEncoder::new()))
            .build("127.0.0.1:9091");

        Config::builder()
            .appender(Appender::builder().build("fluentd", Box::new(fluentd)))
            .logger(Logger::builder().build(facility, LevelFilter::Info))
            .build(Root::builder().appender("fluentd").build(LevelFilter::Info))
    } else {
        let console = ConsoleAppender::builder().build();

        Config::builder()
            .appender(Appender::builder().build("console", Box::new(console)))
            .logger(Logger::builder().build(facility, LevelFilter::Info))
            .build(Root::builder().appender("console").build(LevelFilter::Info))
    }?;

    let handle = log4rs::init_config(config)?;

    Ok(handle)
}

/// Creates a logger instance from log4rs file
///
/// Example log file:
/// ```yaml
/// refresh_rate: 30 seconds
/// appenders:
///   stdout:
///     kind: console
///   fluentd:
///     kind: fluentd
///     facility: mclogger
///     addr: 127.0.0.1:9091
///     encoder:
///       kind: json
/// root:
///   level: info
///   appenders:
///     - fluentd
/// loggers:
///   mclogger:
///     level: info
/// ```
///
/// Usage:
/// ```
/// # #[macro_use]
/// # extern crate log;
/// # fn main() {
/// let _ = vinted_logger::from_file("path/to/log4rs.config.yaml");
/// info!("Log some stuff");
/// # }
/// ```
pub fn from_file(file: impl Into<String>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut deserializers = log4rs::config::Deserializers::default();

    deserializers.insert("fluentd", FluentdAppenderDeserializer);

    log4rs::init_file(file.into(), deserializers)?;

    Ok(())
}
