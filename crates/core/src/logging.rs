//! Structured log format.
//!
//! One JSON line per event with a fixed set of fields:
//! `ts, level, service, env, event, traceId, msg` plus the call's own fields. A shared format is
//! not about looks: a defect is rarely confined to one service. A single user action may pass
//! through several services that each call this one for unit conversion, and the only way to
//! stitch their logs together is a common identifier: timestamps do not work, hundreds of other
//! calls happen in the same millisecond.
//!
//! The stock `tracing_subscriber::fmt().json()` writes `timestamp`/`fields.message`/`target` and
//! supports neither constant fields (`service`, `env`) nor a flat layout. Different field names
//! across services mean a separate parser per service and no single query, hence a custom
//! formatter.

use std::collections::BTreeMap;
use std::fmt;

use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::fmt::format::{JsonFields, Writer};
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields, FormattedFields};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::EnvFilter;

/// Initialise process logging. `service` is the binary's name (`currency-manager`, `currency-sync`, ...).
pub fn init(service: &'static str) {
    let env = std::env::var("OPERATOR").unwrap_or_else(|_| "aggregator".to_string());

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,currency=info".into()),
        )
        // Span fields are encoded as JSON: the event pulls them out of `FormattedFields` and lays
        // them out flat in the line. With the default formatter they would hold `traceId=trace-1`,
        // i.e. text that would need a regex to parse.
        .fmt_fields(JsonFields::new())
        .event_format(FleetJson { service, env })
        .init();
}

/// Request trace: the id under which one and the same request is visible in the logs of every
/// service it touches.
///
/// `x-request-id` is created at the edge, propagated by upstream callers and arrives here in the
/// call metadata. A span with this field is opened on entry, so everything the handler logs comes
/// out with the same id. Without it, correlating a user-facing failure with its root cause is only
/// possible by timestamps, which in practice means not at all.
pub const TRACE_HEADER: &str = "x-request-id";

/// Call span. Opened on entry to the handler and held until it returns.
pub fn call_span(rpc: &'static str, trace_id: Option<&str>) -> tracing::Span {
    match trace_id {
        Some(id) => tracing::info_span!("call", rpc, traceId = id),
        None => tracing::info_span!("call", rpc),
    }
}

pub(crate) struct FleetJson {
    service: &'static str,
    env: String,
}

impl<S, N> FormatEvent<S, N> for FleetJson
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let mut fields = FieldCollector::default();
        event.record(&mut fields);

        let mut line = serde_json::Map::new();
        line.insert("ts".into(), serde_json::Value::String(now_iso8601()));
        line.insert(
            "level".into(),
            serde_json::Value::String(event.metadata().level().to_string()),
        );
        line.insert(
            "service".into(),
            serde_json::Value::String(self.service.to_string()),
        );
        line.insert("env".into(), serde_json::Value::String(self.env.clone()));
        line.insert(
            "msg".into(),
            serde_json::Value::String(fields.message.unwrap_or_default()),
        );
        line.insert(
            "logger".into(),
            serde_json::Value::String(event.metadata().target().to_string()),
        );

        // Fields of the active spans are the call context (traceId and the like): they must land
        // flat in the line, otherwise querying by traceId would require knowing which span holds it.
        if let Some(scope) = ctx.event_scope() {
            for span in scope.from_root() {
                if let Some(formatted) = span.extensions().get::<FormattedFields<N>>() {
                    if let Ok(serde_json::Value::Object(map)) =
                        serde_json::from_str::<serde_json::Value>(formatted.fields.as_str())
                    {
                        for (key, value) in map {
                            line.insert(key, value);
                        }
                    }
                }
            }
        }

        for (key, value) in fields.rest {
            line.insert(key, value);
        }

        writeln!(writer, "{}", serde_json::Value::Object(line))
    }
}

#[derive(Default)]
struct FieldCollector {
    message: Option<String>,
    rest: BTreeMap<String, serde_json::Value>,
}

impl Visit for FieldCollector {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        } else {
            self.rest.insert(
                field.name().to_string(),
                serde_json::Value::String(value.to_string()),
            );
        }
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.rest
            .insert(field.name().to_string(), serde_json::Value::from(value));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.rest
            .insert(field.name().to_string(), serde_json::Value::from(value));
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.rest
            .insert(field.name().to_string(), serde_json::Value::Bool(value));
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        let rendered = format!("{value:?}");
        if field.name() == "message" {
            self.message = Some(rendered);
        } else {
            self.rest.insert(
                field.name().to_string(),
                serde_json::Value::String(rendered),
            );
        }
    }
}

/// No dependency on `time`/`chrono`: epoch seconds rendered as ISO-8601 UTC.
fn now_iso8601() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs() as i64;
    let millis = now.subsec_millis();

    let days = secs.div_euclid(86_400);
    let time = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);

    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        time / 3600,
        (time % 3600) / 60,
        time % 60,
    )
}

/// Howard Hinnant's civil_from_days: calendar conversion without dependencies or tables.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::{Arc, Mutex};

    /// The log format is a contract: the log collector reads it, queries are built on it and
    /// defects are hunted through it. One formatter edit can break it, invisibly to the eye.
    #[derive(Clone, Default)]
    struct Buffer(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for Buffer {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Buffer {
        type Writer = Buffer;

        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    #[test]
    fn line_is_json_with_the_fleet_fields() {
        let buffer = Buffer::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(buffer.clone())
            .fmt_fields(JsonFields::new())
            .event_format(FleetJson {
                service: "currency-manager",
                env: "aggregator".to_string(),
            })
            .finish();

        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!("call", traceId = "trace-1");
            let _entered = span.enter();
            tracing::info!(event = "convert.units", currency = "UAH", "Converted");
        });

        let written = buffer.0.lock().unwrap().clone();
        let line = String::from_utf8(written).unwrap();
        let json: serde_json::Value = serde_json::from_str(line.trim()).unwrap();

        assert_eq!(json["service"], "currency-manager");
        assert_eq!(json["env"], "aggregator");
        assert_eq!(json["level"], "INFO");
        assert_eq!(json["msg"], "Converted");
        assert_eq!(json["event"], "convert.units");
        assert_eq!(json["currency"], "UAH");
        // The span field MUST land FLAT: otherwise a query by traceId would depend on which span
        // declared it.
        assert_eq!(json["traceId"], "trace-1");
        assert!(json["ts"].as_str().unwrap().contains('T'));
    }
}
