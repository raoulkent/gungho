use tracing::info;

#[allow(dead_code)]
mod config;

#[allow(dead_code)]
mod backend;

#[allow(dead_code)]
mod lb;

#[allow(dead_code)]
mod metrics;

#[allow(dead_code)]
mod proxy;

#[allow(dead_code)]
mod health;

#[allow(dead_code)]
mod admin;

#[allow(dead_code)]
mod logging;

#[allow(dead_code)]
mod tracing_otel;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let default_config = config::Config::default();
    let tracer_provider =
        tracing_otel::init_tracer(&default_config.tracing).expect("Failed to init tracer");

    logging::init_logging("info", logging::LogFormat::Pretty, tracer_provider.as_ref())?;
    info!("Hello, world!");

    if let Some(tracer_provider) = tracer_provider {
        tracing_otel::shutdown_tracer(&tracer_provider)?;
    }

    Ok(())
}
