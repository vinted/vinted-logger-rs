use std::error::Error;
use tracing_subscriber::EnvFilter;

pub(crate) mod vinted_json_formatter;
pub(crate) mod vinted_udp_writer;

/// Logging target
#[derive(Debug)]
pub enum Target {
    /// Messages will be logged as JSON and sent to a UDP socket
    UdpJson,

    /// Messages will be logged as JSON to stdout
    ConsoleJson,

    /// Messages will be logged to stdout
    Console,
}

/// Creates an instance of Vinted logger
///
/// - `facility` - facility name, usually the name of the service, e.g. `svc-search`, `core`
pub fn try_init(
    facility: &'static str,
    target: Target,
) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
    match target {
        Target::UdpJson => tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::from_default_env())
            .with_writer(vinted_udp_writer::VintedUdpWriter::new("127.0.0.1:9091"))
            .fmt_fields(tracing_subscriber::fmt::format::JsonFields::new())
            .event_format(vinted_json_formatter::VintedJson::new(facility))
            .try_init(),
        Target::ConsoleJson => tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::from_default_env())
            .fmt_fields(tracing_subscriber::fmt::format::JsonFields::new())
            .event_format(vinted_json_formatter::VintedJson::new(facility))
            .try_init(),
        Target::Console => tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::from_default_env())
            .try_init(),
    }
}
