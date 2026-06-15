use opentelemetry::global;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::trace::SdkTracerProvider;
use std::time::Duration;

use crate::config::TracingConfig;

fn init_tracer(config: &TracingConfig) -> SdkTracerProvider {
    let endpoint = &config.endpoint;

    let resource = Resource::builder().with_service_name("gungho").build();

    // Traces
    let span_exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint.clone())
        .with_timeout(Duration::from_secs(5))
        .build()
        .expect("Failed to build SpanExporter");
    let tracer_provider = SdkTracerProvider::builder()
        .with_resource(resource)
        .with_batch_exporter(span_exporter)
        .build();

    // Set provider to be used as global tracer provider
    global::set_tracer_provider(tracer_provider.clone());

    tracer_provider
}

fn shutdown_tracer(provider: &SdkTracerProvider) {
    provider.force_flush().expect("Failed to force flush");
    provider
        .shutdown()
        .expect("Failed to shutdown TracerProvider");
}
