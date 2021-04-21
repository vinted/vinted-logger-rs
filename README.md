# Vinted Rust logger

Structured logger for Vinted Rust applications. Can log in the following ways:

- Sends logs formatted as JSON messages to a UDP socket. Intended for current production uses.
- Sends logs formatted as JSON messages to a stdout. Intended for containers running in Kubernetes.
- Sends plain logs to stdout. Intended for local development.

## Using in applications

This crate depends on [`tracing`](https://docs.rs/tracing/0.1.25/tracing/) crate, import it to your crate. In `Cargo.toml`:

```toml
tracing = "0.1"
```

And then add this crate to your executable crate:

```toml
vinted-logger = { git = "https://github.com/vinted/vinted-logger-rs" }
```

Logger is initialized from your `main` method.

To add console logger:

```rust
let _ = vinted_logger::try_init("console", vinted_logger::Target::Console);
```

To add console JSON logger:

```rust
let _ = vinted_logger::try_init("console", vinted_logger::Target::ConsoleJson);
```

To add UDP JSON logger:

```rust
let _ = vinted_logger::try_init("console", vinted_logger::Target::UdpJson);
```

## Usage examples

Simple logging:

```rust
info!("Some message");
trace!("Some message");
error!("Some message");
debug!("Some message");
warn!("Some message");
log!("Some message");
```

Structured logging:

```rust
info!(foo="bar", "Some message");
trace!(foo="bar", "Some message");
error!(foo="bar", "Some message");
debug!(foo="bar", "Some message");
warn!(foo="bar", "Some message");
log!(foo="bar", "Some message");
```

Messages are produced in the following JSON format:

```json
{
  "@timestamp": "2021-04-20T12:42:57.353066+00:00",
  "level": "INFO",
  "facility": "console",
  "message": "Binding to http://0.0.0.0:9550",
  "target": "svc_search",
  "thread_id": "ThreadId(1)",
  "thread_name": "main",
  "file": "bin/src/main.rs",
  "module": "svc_search",
  "line": 73,
  "host": "localhost"
}
```
