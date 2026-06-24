use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::trace::{Sampler, SdkTracerProvider};
use std::time::Duration;

use crate::config::TracingConfig;

pub fn init_tracer(config: &TracingConfig) -> anyhow::Result<Option<SdkTracerProvider>> {
    if !config.enabled {
        return Ok(None);
    }
    let endpoint = &config.endpoint;

    let resource = Resource::builder().with_service_name("gungho").build();

    let sampler = Sampler::ParentBased(Box::new(Sampler::TraceIdRatioBased(config.sample_rate)));

    // Traces
    let span_exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint.clone())
        .with_timeout(Duration::from_secs(5))
        .build()?;

    let tracer_provider = SdkTracerProvider::builder()
        .with_resource(resource)
        .with_batch_exporter(span_exporter)
        .with_sampler(sampler)
        .build();

    Ok(Some(tracer_provider))
}

pub fn shutdown_tracer(provider: &SdkTracerProvider) -> anyhow::Result<()> {
    provider.force_flush()?;
    provider.shutdown()?;

    Ok(())
}
