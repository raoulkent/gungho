use hyper::StatusCode;
use prometheus::{HistogramVec, IntCounterVec, IntGauge, IntGaugeVec, Registry};
use prometheus::{histogram_opts, opts};

#[derive(Debug)]
pub struct GunghoMetrics {
    requests_total: IntCounterVec,
    request_duration_seconds: HistogramVec,
    active_connections: IntGauge,
    backend_health: IntGaugeVec,
    backends_total: IntGauge,
    config_reload_total: IntCounterVec,
    registry: Registry,
}

#[derive(Debug, Clone, Copy)]
pub enum ReloadResult {
    Success,
    Failure,
}

impl GunghoMetrics {
    pub fn new() -> Self {
        // Create a new Prometheus registry
        let registry = Registry::new();

        // Initialize Prometheus metrics here
        let requests_total = IntCounterVec::new(
            opts!("gungho_requests_total", "Total number of requests"),
            &["backend", "status_code"],
        )
        .expect("Failed to create requests_total metric");

        let request_duration_seconds = HistogramVec::new(
            histogram_opts!(
                "gungho_request_duration_seconds",
                "Request duration in seconds",
            ),
            &["backend"],
        )
        .expect("Failed to create request_duration_seconds metric");

        let active_connections = IntGauge::new(
            "gungho_active_connections",
            "Current number of active connections",
        )
        .expect("Failed to create active_connections metric");

        let backend_health = IntGaugeVec::new(
            opts!("gungho_backend_health", "Health status of backends"),
            &["backend"],
        )
        .expect("Failed to create backend_health metric");

        let backends_total = IntGauge::new(
            "gungho_backends_total",
            "Total number of backends configured",
        )
        .expect("Failed to create backends_total metric");

        let config_reload_total = IntCounterVec::new(
            opts!(
                "gungho_config_reload_total",
                "Total number of configuration reloads",
            ),
            &["result"],
        )
        .expect("Failed to create config_reload_total metric");

        // Register metrics with the registry
        registry
            .register(Box::new(requests_total.clone()))
            .expect("Failed to register requests_total");
        registry
            .register(Box::new(request_duration_seconds.clone()))
            .expect("Failed to register request_duration_seconds");
        registry
            .register(Box::new(active_connections.clone()))
            .expect("Failed to register active_connections");
        registry
            .register(Box::new(backend_health.clone()))
            .expect("Failed to register backend_health");
        registry
            .register(Box::new(backends_total.clone()))
            .expect("Failed to register backends_total");
        registry
            .register(Box::new(config_reload_total.clone()))
            .expect("Failed to register config_reload_total");

        Self {
            requests_total,
            request_duration_seconds,
            active_connections,
            backend_health,
            backends_total,
            config_reload_total,
            registry,
        }
    }

    pub fn record_request(&self, status_code: StatusCode, backend: &str, duration: f64) {
        self.requests_total
            .with_label_values(&[backend, status_code.as_str()])
            .inc();

        self.request_duration_seconds
            .with_label_values(&[backend])
            .observe(duration);
    }

    pub fn set_active_connections(&self, count: i64) {
        self.active_connections.set(count);
    }

    pub fn set_backend_health(&self, backend: &str, healthy: bool) {
        self.backend_health
            .with_label_values(&[backend])
            .set(i64::from(healthy));
    }

    pub fn record_config_reload(&self, success: ReloadResult) {
        self.config_reload_total
            .with_label_values(&[match success {
                ReloadResult::Success => "success",
                ReloadResult::Failure => "failure",
            }])
            .inc();
    }

    #[allow(clippy::missing_const_for_fn)]
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    pub fn encode_metrics(&self) -> String {
        let text_encoder = prometheus::TextEncoder::new();
        let metrics = self.registry.gather();

        text_encoder
            .encode_to_string(metrics.as_slice())
            .expect("Failed to encode metrics")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static METRIC_NAMES: [&str; 6] = [
        "gungho_requests_total",
        "gungho_request_duration_seconds",
        "gungho_active_connections",
        "gungho_backend_health",
        "gungho_backends_total",
        "gungho_config_reload_total",
    ];

    #[test]
    fn test_counter_increments() {
        let metrics = GunghoMetrics::new();
        metrics.record_request(StatusCode::OK, "backend1", 0.5);

        assert_eq!(
            metrics
                .requests_total
                .with_label_values(&["backend1", "200"])
                .get(),
            1
        );

        for _ in 0..5 {
            metrics.record_request(StatusCode::INTERNAL_SERVER_ERROR, "backend1", 1.0);
        }

        assert_eq!(
            metrics
                .requests_total
                .with_label_values(&["backend1", "500"])
                .get(),
            5
        );
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn test_histogram_records() {
        let metrics = GunghoMetrics::new();

        metrics.record_request(StatusCode::OK, "backend1", 0.5);
        metrics.record_request(StatusCode::OK, "backend1", 1.5);

        let histogram = metrics
            .request_duration_seconds
            .with_label_values(&["backend1"]);

        assert_eq!(histogram.get_sample_count(), 2); // two observations
        assert_eq!(histogram.get_sample_sum(), 2.0); // 0.5 + 1.5
    }

    #[test]
    fn test_encode_metrics_format() {
        let metrics = GunghoMetrics::new();

        metrics.record_request(StatusCode::OK, "backend1", 0.5);
        metrics.set_active_connections(1);
        metrics.set_backend_health("backend1", true);
        metrics.record_config_reload(ReloadResult::Success);

        let encoded = metrics.encode_metrics();

        for name in &METRIC_NAMES {
            assert!(
                encoded.contains(name),
                "Encoded metrics should contain metric name: {name}"
            );
            assert!(
                encoded.contains(&format!("# TYPE {name} ")),
                "Encoded metrics should contain # TYPE line for metric: {name}",
            );
            assert!(
                encoded.contains(&format!("# HELP {name}")),
                "Encoded metrics should contain # HELP line for metric: {name}",
            );
        }
    }

    #[test]
    fn test_all_metrics_registered() {
        let metrics = GunghoMetrics::new();
        metrics.record_request(StatusCode::OK, "backend1", 0.5);
        metrics.set_active_connections(1);
        metrics.set_backend_health("backend1", true);
        metrics.record_config_reload(ReloadResult::Success);

        // Assert that at least 6 metrics are registered (the ones we defined)
        assert!(
            metrics.registry().gather().len() >= 6,
            "Expected at least 6 metrics to be registered"
        );

        let families = metrics.registry().gather();
        let names: Vec<&str> = families
            .iter()
            .map(prometheus::proto::MetricFamily::name)
            .collect();

        // Assert that all defined metric names are present in the registry
        for name in &METRIC_NAMES {
            assert!(
                names.contains(name),
                "Registry should contain metric: {name}"
            );
        }
    }
}
