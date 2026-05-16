use crate::backend::BackendPool;
use crate::config::HealthCheckConfig;
use crate::metrics::GunghoMetrics;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::Empty;
use hyper_util::client::legacy::connect::HttpConnector;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

enum HealthCheckResult {
    Healthy,
    Unhealthy,
}

struct BackendStatus {
    consecutive_successes: u32,
    consecutive_failures: u32,
}

impl BackendStatus {
    const fn update(&mut self, result: &HealthCheckResult) {
        match result {
            HealthCheckResult::Healthy => {
                self.consecutive_successes += 1;
                self.consecutive_failures = 0;
            }
            HealthCheckResult::Unhealthy => {
                self.consecutive_successes = 0;
                self.consecutive_failures += 1;
            }
        }
    }
}

pub struct HealthChecker {
    config: HealthCheckConfig,
    pool: Arc<BackendPool>,
    metrics: Arc<GunghoMetrics>,
    cancellation_token: CancellationToken,
    client: hyper_util::client::legacy::Client<HttpConnector, Empty<Bytes>>,
}

impl HealthChecker {
    pub(crate) fn new(
        config: HealthCheckConfig,
        pool: Arc<BackendPool>,
        metrics: Arc<GunghoMetrics>,
        cancellation_token: CancellationToken,
    ) -> Self {
        let client =
            hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
                .build_http();
        Self {
            config,
            pool,
            metrics,
            cancellation_token,
            client,
        }
    }

    pub(crate) async fn run(self) {
        let mut status_map: HashMap<SocketAddr, BackendStatus> = HashMap::new();
        let mut interval = tokio::time::interval(Duration::from_secs(self.config.interval));

        loop {
            tokio::select! {
                () = self.cancellation_token.cancelled() => break,
                _ = interval.tick() => {
                    let mut set: JoinSet<(SocketAddr, HealthCheckResult)> = JoinSet::new();
                    for backend in self.pool.all_backends() {
                        let client = self.client.clone();  // cheap clone
                        let path = self.config.path.clone();
                        let timeout = self.config.timeout;
                        let addr = backend.get_addr();
                        set.spawn(async move {
                            Self::check(client, addr, path, timeout).await
                        });
                    }
                    while let Some(Ok((addr, health_result))) = set.join_next().await {
                        // Get status or set it to default
                        let status = status_map
                            .entry(addr)
                            .or_insert(BackendStatus { consecutive_successes: 0, consecutive_failures: 0 });

                        // Update the counter
                        status.update(&health_result);

                        // Check threshholds
                        // If failures surpasses threshhold and backend was healthy, mark unhealthy
                        if status.consecutive_failures >= self.config.unhealthy_threshold
                            && self.pool.get_health_by_addr(addr) == Some(true)
                        {
                            self.pool.mark_unhealthy_by_addr(addr);
                            tracing::warn!("Backend {} is unhealthy", addr);
                            self.metrics.set_backend_health(&addr.to_string(), false);
                        }
                        // If successes surpasses threshhold and backend was unhealthy, mark healthy and log
                        if status.consecutive_successes >= self.config.healthy_threshold
                            && self.pool.get_health_by_addr(addr) == Some(false)
                        {
                            self.pool.mark_healthy_by_addr(addr);
                            tracing::info!("Backend {} is healthy", addr);
                            self.metrics.set_backend_health(&addr.to_string(), true);
                        }
                    }
                }
            }
        }
    }

    async fn check(
        client: hyper_util::client::legacy::Client<HttpConnector, Empty<Bytes>>,
        addr: SocketAddr,
        path: String,
        timeout: u64,
    ) -> (SocketAddr, HealthCheckResult) {
        let full_uri = format!("http://{addr}{path}");
        let body: Empty<Bytes> = Empty::new();
        let req = hyper::Request::builder()
            .method("GET")
            .uri(full_uri)
            .body(body)
            .expect("Failed to build request");

        match tokio::time::timeout(Duration::from_secs(timeout), client.request(req)).await {
            Ok(Ok(response)) => {
                if response.status() == hyper::StatusCode::OK {
                    (addr, HealthCheckResult::Healthy)
                } else {
                    (addr, HealthCheckResult::Unhealthy)
                }
            }
            Ok(Err(_)) | Err(_) => (addr, HealthCheckResult::Unhealthy),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;
    use hyper::StatusCode;
    use pretty_assertions::assert_eq;

    use crate::config::BackendConfig;
    use crate::metrics::GunghoMetrics;

    fn setup_pool(configs: &[BackendConfig]) -> BackendPool {
        BackendPool::from_config(configs).expect("Failed to create BackendPool")
    }

    // Setup of backend that returns a given http status code
    async fn spawn_mock_backend(status: Arc<Mutex<StatusCode>>) -> SocketAddr {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("Failed to bind");
        let addr = listener.local_addr().expect("Failed to read listener addr");

        // TODO: spawn a task that loops accepting connections
        tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener
                    .accept()
                    .await
                    .expect("Failed to accept connection");

                // read req, write resp with status code

                let buf: &mut [u8] = &mut [0; 1024];
                let _req = AsyncReadExt::read(&mut stream, buf)
                    .await
                    .expect("Failed to read");

                let code = *status.lock().expect("Failed to lock status code");
                let resp = format!(
                    "HTTP/1.1 {} {}\r\nContent-Length: 0\r\n\r\n",
                    code.as_u16(),
                    code.canonical_reason().unwrap_or("")
                );

                AsyncWriteExt::write_all(&mut stream, resp.as_bytes())
                    .await
                    .expect("Failed to write");
            }
        });

        addr
    }

    // 1. test_healthy_backend_stays_healthy — backend returning 200 stays healthy
    #[tokio::test]
    async fn test_healthy_backend_stays_healthy() {
        let addr = spawn_mock_backend(Arc::new(Mutex::new(StatusCode::OK))).await;
        let pool = Arc::new(setup_pool(&[BackendConfig {
            addr: addr.to_string(),
            weight: 1,
        }]));
        let token = CancellationToken::new();
        let checker = HealthChecker::new(
            HealthCheckConfig {
                interval: 1,
                healthy_threshold: 1,
                unhealthy_threshold: 1,
                timeout: 3,
                path: String::from("/health"),
            },
            pool.clone(),
            Arc::new(GunghoMetrics::new()),
            token.clone(),
        );
        tokio::spawn(checker.run());
        // Wait enough ticks for thresholds to matter
        tokio::time::sleep(Duration::from_secs(2)).await;
        token.cancel();
        assert_eq!(pool.get_health_by_addr(addr), Some(true));
    }

    // 2. test_unhealthy_after_threshold — backend returning 500, marked unhealthy after N failures
    #[tokio::test]
    async fn test_unhealthy_after_threshold() {
        let addr =
            spawn_mock_backend(Arc::new(Mutex::new(StatusCode::INTERNAL_SERVER_ERROR))).await;
        let pool = Arc::new(setup_pool(&[BackendConfig {
            addr: addr.to_string(),
            weight: 1,
        }]));
        let token = CancellationToken::new();
        let checker = HealthChecker::new(
            HealthCheckConfig {
                interval: 1,
                healthy_threshold: 1,
                unhealthy_threshold: 1,
                timeout: 3,
                path: String::from("/health"),
            },
            pool.clone(),
            Arc::new(GunghoMetrics::new()),
            token.clone(),
        );
        tokio::spawn(checker.run());
        // Wait enough ticks for thresholds to matter
        tokio::time::sleep(Duration::from_secs(2)).await;
        token.cancel();
        assert_eq!(pool.get_health_by_addr(addr), Some(false));
    }

    // 3. test_backend_recovery — unhealthy backend starts returning 200, recovers after threshold
    #[tokio::test]
    async fn test_backend_recovery() {
        let status_code: Arc<Mutex<StatusCode>> =
            Arc::new(Mutex::new(StatusCode::INTERNAL_SERVER_ERROR));

        let addr = spawn_mock_backend(status_code.clone()).await;
        let pool = Arc::new(setup_pool(&[BackendConfig {
            addr: addr.to_string(),
            weight: 1,
        }]));
        let token = CancellationToken::new();
        let checker = HealthChecker::new(
            HealthCheckConfig {
                interval: 1,
                healthy_threshold: 1,
                unhealthy_threshold: 1,
                timeout: 3,
                path: String::from("/health"),
            },
            pool.clone(),
            Arc::new(GunghoMetrics::new()),
            token.clone(),
        );
        tokio::spawn(checker.run());
        // Wait enough ticks for thresholds to matter
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert_eq!(pool.get_health_by_addr(addr), Some(false));

        *status_code.lock().expect("Failed to lock status code") = StatusCode::OK;
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert_eq!(pool.get_health_by_addr(addr), Some(true));

        token.cancel();
    }

    // 4. test_unreachable_backend — connection refused counts as failure
    #[tokio::test]
    async fn test_unreachable_backend() {
        let addr: SocketAddr = "127.0.0.1:1".parse().expect("Could not parse SocketAddr");
        let pool = Arc::new(setup_pool(&[BackendConfig {
            addr: addr.to_string(),
            weight: 1,
        }]));
        let token = CancellationToken::new();
        let checker = HealthChecker::new(
            HealthCheckConfig {
                interval: 1,
                healthy_threshold: 1,
                unhealthy_threshold: 1,
                timeout: 3,
                path: String::from("/health"),
            },
            pool.clone(),
            Arc::new(GunghoMetrics::new()),
            token.clone(),
        );
        tokio::spawn(checker.run());
        // Wait enough ticks for thresholds to matter
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert_eq!(pool.get_health_by_addr(addr), Some(false));

        token.cancel();
    }

    // 5. test_respects_cancellation — stops checking when token cancelled
    #[tokio::test]
    async fn test_respects_cancellation() {
        let status_code: Arc<Mutex<StatusCode>> = Arc::new(Mutex::new(StatusCode::OK));

        let addr = spawn_mock_backend(status_code.clone()).await;
        let pool = Arc::new(setup_pool(&[BackendConfig {
            addr: addr.to_string(),
            weight: 1,
        }]));
        let token = CancellationToken::new();
        let checker = HealthChecker::new(
            HealthCheckConfig {
                interval: 1,
                healthy_threshold: 1,
                unhealthy_threshold: 1,
                timeout: 3,
                path: String::from("/health"),
            },
            pool.clone(),
            Arc::new(GunghoMetrics::new()),
            token.clone(),
        );
        let handle = tokio::spawn(checker.run());
        // Wait enough ticks for thresholds to matter
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert_eq!(pool.get_health_by_addr(addr), Some(true));

        token.cancel();
        handle.await.expect("Health checker task panicked");

        *status_code.lock().expect("Could not lock status code") =
            StatusCode::INTERNAL_SERVER_ERROR;
        assert_eq!(pool.get_health_by_addr(addr), Some(true));
    }
}
