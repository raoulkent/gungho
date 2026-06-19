use opentelemetry_sdk::trace::SdkTracerProvider;
use serde::Deserialize;

use std::env::{VarError, var};
use tracing::Subscriber;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{EnvFilter, fmt};

const DEFAULT_LOG_LEVEL: &str = "debug";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LogFormat {
    #[default]
    Pretty,
    Json,
}

fn build_subscriber(log_level: &str, format: LogFormat) -> anyhow::Result<impl Subscriber> {
    let filter = build_filter(log_level)?;
    let fmt_layer = match format {
        LogFormat::Pretty => fmt::layer().pretty().boxed(),
        LogFormat::Json => fmt::layer().json().boxed(),
    };
    let otel_layer = tracing_opentelemetry::layer().boxed();

    Ok(tracing_subscriber::registry()
        .with(filter)
        .with(fmt_layer)
        .with(otel_layer))
}

pub fn init_logging(
    log_level: &str,
    format: LogFormat,
    _tracer_provider: &SdkTracerProvider,
) -> anyhow::Result<()> {
    let subscriber = build_subscriber(log_level, format)?;
    subscriber.try_init()?;
    Ok(())
}

/// This function is used to build the filter for the log level.
fn build_filter(log_level: &str) -> anyhow::Result<EnvFilter> {
    match var("RUST_LOG") {
        Ok(value) => EnvFilter::try_new(value).map_err(Into::into),
        Err(VarError::NotPresent) => EnvFilter::try_new(log_level)
            .or_else(|_| EnvFilter::try_new(DEFAULT_LOG_LEVEL))
            .map_err(Into::into),
        Err(err) => Err(err.into()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_build_subscriber_doesnt_panic() {
        let _ = super::build_subscriber("info", super::LogFormat::Pretty);
    }
}
