use serde::ser::{SerializeMap, Serializer as _};
use serde_json::Serializer;
use std::{fmt, io};
use tracing_core::{Event, Subscriber};
use tracing_serde::AsSerde;
use tracing_subscriber::{
    fmt::{
        format::{FormatEvent, FormatFields},
        time::FormatTime,
        FmtContext, FormattedFields,
    },
    registry::LookupSpan,
};
#[derive(Debug)]
pub(crate) struct VintedJson {
    facility: &'static str,
}
impl VintedJson {
    pub(crate) fn new(facility: &'static str) -> Self {
        Self { facility }
    }
}
impl<S, N> FormatEvent<S, N> for VintedJson
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
    N: for<'writer> FormatFields<'writer> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        writer: &mut dyn fmt::Write,
        event: &Event<'_>,
    ) -> fmt::Result
    where
        S: Subscriber + for<'a> LookupSpan<'a>,
    {
        let mut timestamp = String::new();
        tracing_subscriber::fmt::time::ChronoUtc::rfc3339().format_time(&mut timestamp)?;
        let meta = event.metadata();
        let mut visit = || {
            let format_field_marker: std::marker::PhantomData<N> = std::marker::PhantomData;
            let mut serializer = Serializer::new(WriteAdaptor::new(writer));
            let mut serializer = serializer.serialize_map(None)?;
            serializer.serialize_entry("@timestamp", &timestamp)?;
            serializer.serialize_entry("level", &meta.level().as_serde())?;
            serializer.serialize_entry("facility", self.facility)?;
            let current_span = event
                .parent()
                .and_then(|id| ctx.span(id))
                .or_else(|| ctx.lookup_current());
            let mut visitor = tracing_serde::SerdeMapVisitor::new(serializer);
            event.record(&mut visitor);
            serializer = visitor.take_serializer()?;
            serializer.serialize_entry("target", meta.target())?;
            if let Some(ref span) = current_span {
                serializer
                    .serialize_entry("spans", &SerializableContext(ctx, format_field_marker))?;
                serializer
                    .serialize_entry("span", &SerializableSpan(span, format_field_marker))
                    .unwrap_or(());
            }
            let current_thread = std::thread::current();
            serializer.serialize_entry("thread_id", &format!("{:?}", current_thread.id()))?;
            if let Some(thread_name) = current_thread.name() {
                serializer.serialize_entry("thread_name", thread_name)?;
            }
            if let Some(file) = meta.file() {
                serializer.serialize_entry("file", file)?;
            }
            if let Some(module) = meta.module_path() {
                serializer.serialize_entry("module", module)?;
            }
            if let Some(ref line) = meta.line() {
                serializer.serialize_entry("line", line)?;
            }
            if let Some(hostname) = gethostname::gethostname().to_str() {
                serializer.serialize_entry("host", hostname)?;
            }
            serializer.end()
        };
        visit().map_err(|_| fmt::Error)?;
        writeln!(writer)
    }
}
struct SerializableContext<'a, 'b, Span, N>(
    &'b tracing_subscriber::fmt::FmtContext<'a, Span, N>,
    std::marker::PhantomData<N>,
)
where
    Span: Subscriber + for<'lookup> tracing_subscriber::registry::LookupSpan<'lookup>,
    N: for<'writer> FormatFields<'writer> + 'static;
impl<'a, 'b, Span, N> serde::ser::Serialize for SerializableContext<'a, 'b, Span, N>
where
    Span: Subscriber + for<'lookup> tracing_subscriber::registry::LookupSpan<'lookup>,
    N: for<'writer> FormatFields<'writer> + 'static,
{
    fn serialize<Ser>(&self, serializer_o: Ser) -> Result<Ser::Ok, Ser::Error>
    where
        Ser: serde::ser::Serializer,
    {
        use serde::ser::SerializeSeq;
        let mut serializer = serializer_o.serialize_seq(None)?;
        for span in self.0.scope() {
            serializer.serialize_element(&SerializableSpan(&span, self.1))?;
        }
        serializer.end()
    }
}
struct SerializableSpan<'a, 'b, Span, N>(
    &'b tracing_subscriber::registry::SpanRef<'a, Span>,
    std::marker::PhantomData<N>,
)
where
    Span: for<'lookup> tracing_subscriber::registry::LookupSpan<'lookup>,
    N: for<'writer> FormatFields<'writer> + 'static;
impl<'a, 'b, Span, N> serde::ser::Serialize for SerializableSpan<'a, 'b, Span, N>
where
    Span: for<'lookup> tracing_subscriber::registry::LookupSpan<'lookup>,
    N: for<'writer> FormatFields<'writer> + 'static,
{
    fn serialize<Ser>(&self, serializer: Ser) -> Result<Ser::Ok, Ser::Error>
    where
        Ser: serde::ser::Serializer,
    {
        let mut serializer = serializer.serialize_map(None)?;
        let ext = self.0.extensions();
        let data = ext
            .get::<FormattedFields<N>>()
            .expect("Unable to find FormattedFields in extensions; this is a bug");
        // TODO: let's _not_ do this, but this resolves
        // https://github.com/tokio-rs/tracing/issues/391.
        // We should probably rework this to use a `serde_json::Value` or something
        // similar in a JSON-specific layer, but I'd (david)
        // rather have a uglier fix now rather than shipping broken JSON.
        match serde_json::from_str::<serde_json::Value>(&data) {
            Ok(serde_json::Value::Object(fields)) => {
                for field in fields {
                    serializer.serialize_entry(&field.0, &field.1)?;
                }
            }
            // We have fields for this span which are valid JSON but not an object.
            // This is probably a bug, so panic if we're in debug mode
            Ok(_) if cfg!(debug_assertions) => panic!(
                "span '{}' had malformed fields! this is a bug.\n  error: invalid JSON object\n  fields: {:?}",
                self.0.metadata().name(),
                data
            ),
            // If we *aren't* in debug mode, it's probably best not to
            // crash the program, let's log the field found but also an
            // message saying it's type  is invalid
            Ok(value) => {
                serializer.serialize_entry("field", &value)?;
                serializer.serialize_entry("field_error", "field was no a valid object")?
            }
            // We have previously recorded fields for this span
            // should be valid JSON. However, they appear to *not*
            // be valid JSON. This is almost certainly a bug, so
            // panic if we're in debug mode
            Err(e) if cfg!(debug_assertions) => panic!(
                "span '{}' had malformed fields! this is a bug.\n  error: {}\n  fields: {:?}",
                self.0.metadata().name(),
                e,
                data
            ),
            // If we *aren't* in debug mode, it's probably best not
            // crash the program, but let's at least make sure it's clear
            // that the fields are not supposed to be missing.
            Err(e) => serializer.serialize_entry("field_error", &format!("{}", e))?,
        };
        serializer.serialize_entry("name", self.0.metadata().name())?;
        serializer.end()
    }
}
/// A bridge between `fmt::Write` and `io::Write`.
///
/// This is needed because tracing-subscriber's FormatEvent expects a fmt::Write
/// while `serde_json`'s Serializer expects an io::Write.
struct WriteAdaptor<'a> {
    fmt_write: &'a mut dyn fmt::Write,
}
impl<'a> WriteAdaptor<'a> {
    fn new(fmt_write: &'a mut dyn fmt::Write) -> Self {
        Self { fmt_write }
    }
}
impl<'a> io::Write for WriteAdaptor<'a> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let s =
            std::str::from_utf8(buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        self.fmt_write
            .write_str(&s)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        Ok(s.as_bytes().len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl<'a> fmt::Debug for WriteAdaptor<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad("WriteAdaptor { .. }")
    }
}
