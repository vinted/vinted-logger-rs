pub mod vinted_udp_logger;

#[macro_use]
extern crate tracing;

use vinted_udp_logger::*;

#[tokio::main]
async fn main() {
    from_config("production", "mc-logger");

    // Send a test log
    tracing::info!(
        target = "test target",
        module = "the module",
        file = "Mr. file",
        message = "testing out logging"
    );

    // Don't exit so the thread could finish uploading
    loop {}
}
