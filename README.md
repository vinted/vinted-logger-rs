# Vinted Rust logger

Structured logger for Vinted Rust applications. Acts as simple logger in dev/test environments. In production environment forwards all messages to the local [Fluent Bit](https://fluentbit.io) forwarder (usually localhost:9091).

## Using in applications

### Libraries
Add a crate to your `Cargo.toml`

```toml
tracing = "0.1"
```

### Binaries
```toml
vinted-logger = { git = "https://github.com/vinted/vinted-logger-rs" }
```


In your `main` function, add the following code:

```rust
let _ = vinted_logger::try_init("console", vinted_logger::Target::Udp);
```

## Usage

```rust
info!("Some message");
trace!("Some message");
error!("Some message");
debug!("Some message");
warn!("Some message");
log!("Some message");
```

Messages are produced in the following JSON format:

```json
{
  "facility": "svc_search",
  "host": "vinted",
  "module": "svc_search",
  "file": "src/bin/svc-search.rs",
  "level": "INFO",
  "message": "Some message",
  "@timestamp": "2021-01-11T13:24:50.361397622Z"
}
```

In production environment it will be passed as is to the Fluent Bit logger and eventually will end up in elasticsearch cluster (and Kibana). In development environment it will result in text being logged to console.
