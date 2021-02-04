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

use chrono::{DateTime, Utc};
use log::{LevelFilter, Record};
use log4rs::append::console::ConsoleAppender;
use log4rs::config::{Appender, Logger, Root};
use log4rs::Config;
use serde::{Deserialize, Serialize};
use std::net::ToSocketAddrs;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Mutex;
use std::{fmt::Debug, net::UdpSocket};

#[derive(Serialize)]
struct LogRecord {
    environment: String,
    facility: String,
    host: String,
    target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    module: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    file: Option<String>,
    level: String,
    message: String,
    #[serde(rename = "@timestamp")]
    timestamp: DateTime<Utc>,
}

impl LogRecord {
    #[allow(clippy::too_many_arguments)]
    fn new(
        environment: &str,
        facility: &str,
        host: &str,
        target: &str,
        module: Option<&str>,
        file: Option<&str>,
        level: &str,
        message: &str,
    ) -> Self {
        LogRecord {
            environment: environment.into(),
            facility: facility.into(),
            host: host.into(),
            target: target.into(),
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
    sender: Mutex<Sender<LogRecord>>,
    facility: String,
    host: String,
    environment: String,
}

impl FluentdAppender {
    fn builder() -> FluentdAppenderBuilder {
        FluentdAppenderBuilder {
            facility: "".into(),
            environment: "".into(),
        }
    }
}

impl ::log4rs::append::Append for FluentdAppender {
    fn append(&self, record: &Record) -> Result<(), anyhow::Error> {
        let log_record = LogRecord::new(
            &self.environment,
            &self.facility,
            &self.host,
            record.target(),
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
    facility: String,
    environment: String,
}

impl FluentdAppenderBuilder {
    /// Sets facility
    fn facility(mut self, facility: &str) -> Self {
        self.facility = facility.into();
        self
    }

    /// Sets environment
    fn environment(mut self, environment: &str) -> Self {
        self.environment = environment.into();
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

        //Thread receiving all log_record and sending them to fluentd
        let _ = ::std::thread::spawn(move || {
            match UdpSocket::bind("127.0.0.1:0") {
                Ok(socket) => loop {
                    match receiver.recv() {
                        Ok(log_record) => match serde_json::to_string(&log_record) {
                            Ok(record) => {
                                if let Err(e) = socket.send_to(record.as_bytes(), &addr) {
                                    eprintln!("Log record can't be sent to fluentd: {}", e);
                                }
                            }
                            Err(e) => {
                                eprintln!("Couldn't serialize log record: {}", e);
                            }
                        },
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

        FluentdAppender {
            sender: Mutex::new(sender),
            facility: self.facility,
            environment: self.environment,
            host: gethostname::gethostname()
                .into_string()
                .unwrap_or_else(|_| "unknown".into()),
        }
    }
}

#[derive(Deserialize)]
struct FluentdAppenderConfig {
    addr: String,
    facility: String,
    environment: String,
}

struct FluentdAppenderDeserializer;

impl log4rs::config::Deserialize for FluentdAppenderDeserializer {
    type Trait = dyn log4rs::append::Append;
    type Config = FluentdAppenderConfig;

    fn deserialize(
        &self,
        config: Self::Config,
        _deserializers: &log4rs::config::Deserializers,
    ) -> anyhow::Result<Box<Self::Trait>> {
        let appender = FluentdAppender::builder()
            .facility(&config.facility)
            .environment(&config.environment)
            .build(config.addr);

        Ok(Box::new(appender))
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
/// # extern crate log;
/// # fn main() {
/// let _ = vinted_logger::from_config("production", "mc-logger");
/// info!("Log some stuff");
/// # }
/// ```
pub fn from_config(
    environment: impl AsRef<str>,
    facility: impl AsRef<str>,
) -> Result<log4rs::Handle, Box<dyn std::error::Error + Send + Sync>> {
    let facility = facility.as_ref();
    let environment = environment.as_ref();

    let config = if environment == "production" {
        let fluentd = FluentdAppender::builder()
            .facility(facility)
            .environment(environment)
            .build("127.0.0.1:9091");

        Config::builder()
            .appender(Appender::builder().build("fluentd", Box::new(fluentd)))
            .logger(Logger::builder().build(facility, LevelFilter::Info))
            .build(Root::builder().appender("fluentd").build(LevelFilter::Info))
    } else {
        let console = ConsoleAppender::builder().build();

        Config::builder()
            .appender(Appender::builder().build("console", Box::new(console)))
            .logger(Logger::builder().build(facility, LevelFilter::Trace))
            .build(
                Root::builder()
                    .appender("console")
                    .build(LevelFilter::Trace),
            )
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
///     environment: production
///     facility: svc-logger
///     addr: 127.0.0.1:9091
/// root:
///   level: info
///   appenders:
///     - fluentd
/// loggers:
///   svc-logger:
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
