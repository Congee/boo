//! Runtime tracing initialization.
//!
//! Boo is migrating from the `log` facade to `tracing`.  This module installs a
//! `tracing-subscriber` formatter with `RUST_LOG`/`EnvFilter` syntax, bridges
//! existing `log::*` call sites into the subscriber, and mirrors Rust traces to
//! Apple OSLog on Apple platforms.

use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;

const DEFAULT_FILTER: &str = "info";
#[cfg(target_vendor = "apple")]
const APPLE_OSLOG_SUBSYSTEM: &str = "dev.boo.rust";
#[cfg(target_vendor = "apple")]
const APPLE_OSLOG_CATEGORY: &str = "latency";

/// Initialize process-wide tracing with an optional first-class filter override.
///
/// When `filter` is `Some`, it takes precedence over `RUST_LOG` while using the
/// same directive syntax.  This keeps repeated trace-verification commands
/// approval-rule friendly without losing compatibility with existing
/// `RUST_LOG` workflows.
pub(crate) fn init_with_filter(filter: Option<&str>) {
    let _ = tracing_log::LogTracer::init();

    let subscriber = tracing_subscriber::registry()
        .with(env_filter(filter))
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(true)
                .with_writer(std::io::stderr),
        );

    #[cfg(target_vendor = "apple")]
    let subscriber = subscriber.with(tracing_oslog::OsLogger::new(
        APPLE_OSLOG_SUBSYSTEM,
        APPLE_OSLOG_CATEGORY,
    ));

    let _ = tracing::subscriber::set_global_default(subscriber);
}

fn env_filter(filter: Option<&str>) -> EnvFilter {
    filter
        .and_then(|value| parse_env_filter(value).ok())
        .unwrap_or_else(env_filter_from_rust_log)
}

fn env_filter_from_rust_log() -> EnvFilter {
    std::env::var("RUST_LOG")
        .ok()
        .and_then(|value| parse_env_filter(&value).ok())
        .unwrap_or_else(default_env_filter)
}

fn default_env_filter() -> EnvFilter {
    parse_env_filter(DEFAULT_FILTER).expect("default tracing filter must parse")
}

fn parse_env_filter(value: &str) -> Result<EnvFilter, tracing_subscriber::filter::ParseError> {
    EnvFilter::try_new(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_rust_log_style_filters() {
        assert!(parse_env_filter("boo=debug,remote=trace,warn").is_ok());
    }

    #[test]
    fn rejects_invalid_filters_before_fallback() {
        assert!(parse_env_filter("boo=[definitely-not-a-level]").is_err());
    }

    #[test]
    fn explicit_filter_uses_rust_log_style_directives() {
        let _ = env_filter(Some("boo::latency=info,trace_init=debug"));
    }

    #[test]
    fn invalid_explicit_filter_falls_back() {
        let _ = env_filter(Some("boo=[definitely-not-a-level]"));
    }

    #[cfg(target_vendor = "apple")]
    #[test]
    fn apple_oslog_uses_shared_latency_category() {
        assert_eq!(APPLE_OSLOG_SUBSYSTEM, "dev.boo.rust");
        assert_eq!(APPLE_OSLOG_CATEGORY, "latency");
    }
}
