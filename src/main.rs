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

fn main() -> anyhow::Result<()> {
    logging::init_logging("info", logging::LogFormat::Pretty)?;
    info!("Hello, world!");

    Ok(())
}
