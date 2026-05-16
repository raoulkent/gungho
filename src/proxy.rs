use bytes::Bytes;
use http_body_util::{Either, Full};
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;

use hyper::StatusCode;
use hyper::body::Incoming;
use hyper::http::header::HeaderValue;
use hyper::{HeaderMap, Request, Response};
use tokio::net::TcpListener;
use tokio::time::{Duration, timeout};

use crate::backend::{Backend, BackendPool};
use crate::config::Algorithm;
use crate::lb;
use crate::lb::LoadBalancingStrategy;
use crate::metrics::GunghoMetrics;

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

struct ConnectionGuard {
    backend: Arc<Backend>,
}

impl ConnectionGuard {
    fn new(backend: Arc<Backend>) -> Self {
        backend.increment_connections();
        Self { backend }
    }
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.backend.decrement_connections();
    }
}

struct Proxy {
    listener: TcpListener,
    pool: Arc<BackendPool>,
    strategy: Arc<dyn LoadBalancingStrategy>,
    timeout: Duration,
    metrics: Arc<GunghoMetrics>,
    client: Client<HttpConnector, Incoming>,
}

impl Proxy {
    pub async fn new(
        pool: Arc<BackendPool>,
        algorithm: Algorithm,
        addr: &str,
        timeout: Duration,
    ) -> Self {
        let strategy = Arc::from(lb::create_strategy(&algorithm));
        let metrics = Arc::from(GunghoMetrics::new());
        let listener = TcpListener::bind(addr)
            .await
            .expect("Failed to bind to address");
        let client = Client::builder(hyper_util::rt::TokioExecutor::new()).build_http();

        Self {
            listener,
            pool,
            strategy,
            timeout,
            metrics,
            client,
        }
    }
    pub async fn run(self) {
        let http = hyper::server::conn::http1::Builder::new();
        let this = Arc::new(self);

        loop {
            let (stream, client_addr) = this
                .listener
                .accept()
                .await
                .expect("Failed to accept connection");

            let this = Arc::clone(&this);

            let connection = http.serve_connection(
                hyper_util::rt::TokioIo::new(stream),
                hyper::service::service_fn(move |req| {
                    let this = Arc::clone(&this);
                    async move { this.handle_request(req, client_addr).await }
                }),
            );

            tokio::spawn(async move {
                if let Err(e) = connection.await {
                    tracing::error!("Connection error: {e}");
                }
            });
        }
    }

    fn insert_x_forwarded_headers(
        header_map: &mut HeaderMap<HeaderValue>,
        client_addr: SocketAddr,
        backend_addr: SocketAddr,
    ) {
        // Add X-Forwarded-For, X-Forwarded-Host and X-Forwarded-Proto headers
        header_map.insert(
            "X-Forwarded-For",
            client_addr.ip().to_string().parse().expect("Invalid IP"),
        );
        header_map.insert("X-Forwarded-Proto", "http".parse().expect("Invalid Scheme"));

        let original_host = header_map.get("host").cloned();
        header_map.insert(
            "Host",
            backend_addr.to_string().parse().expect("Invalid Host"),
        );

        if let Some(host) = original_host {
            header_map.insert("X-Forwarded-Host", host);
        }
    }

    fn strip_hop_by_hop_headers(header_map: &mut HeaderMap<HeaderValue>) {
        // Remove hop-by-hop headers as per RFC 2616 Section 13.5.1
        let hop_by_hop_headers = [
            "Connection",
            "Keep-Alive",
            "Proxy-Authenticate",
            "Proxy-Authorization",
            "TE",
            "Trailers",
            "Transfer-Encoding",
            "Upgrade",
        ];

        for header in &hop_by_hop_headers {
            header_map.remove(*header);
        }
    }

    fn build_error_response(
        status: StatusCode,
        string: &'static [u8],
    ) -> Response<Either<Incoming, Full<Bytes>>> {
        Response::builder()
            .status(status)
            .body(Either::Right(Full::new(Bytes::from_static(string))))
            .expect("Failed to build response")
    }

    async fn handle_request(
        self: Arc<Self>,
        mut req: Request<Incoming>,
        client_addr: SocketAddr,
    ) -> Result<Response<Either<Incoming, Full<Bytes>>>, Infallible> {
        let healthy = self.pool.healthy_backends();
        let start = std::time::Instant::now();

        let Some(index) = self.strategy.select(&healthy, Some(&client_addr)) else {
            let status = StatusCode::SERVICE_UNAVAILABLE;
            self.metrics
                .record_request(status, "none", start.elapsed().as_secs_f64());

            return Ok(Self::build_error_response(
                status,
                b"No healthy backends available",
            ));
        };

        let backend = &healthy[index];
        let backend_addr = backend.get_addr();

        *req.uri_mut() = format!("http://{backend_addr}{}", req.uri().path())
            .parse()
            .expect("Failed to parse URI");

        Self::insert_x_forwarded_headers(req.headers_mut(), client_addr, backend_addr);

        Self::strip_hop_by_hop_headers(req.headers_mut());

        let _guard = ConnectionGuard::new(Arc::clone(backend));

        match timeout(self.timeout, self.client.request(req)).await {
            Ok(Ok(mut resp)) => {
                let status = resp.status();
                self.metrics.record_request(
                    status,
                    &backend_addr.to_string(),
                    start.elapsed().as_secs_f64(),
                );

                Self::strip_hop_by_hop_headers(resp.headers_mut());

                Ok(resp.map(Either::Left))
            }
            Ok(Err(_)) => {
                let status = StatusCode::BAD_GATEWAY;
                self.metrics.record_request(
                    status,
                    &backend_addr.to_string(),
                    start.elapsed().as_secs_f64(),
                );

                Ok(Self::build_error_response(status, b"Backend error"))
            }
            Err(_) => {
                let status = StatusCode::GATEWAY_TIMEOUT;
                self.metrics.record_request(
                    status,
                    &backend_addr.to_string(),
                    start.elapsed().as_secs_f64(),
                );

                Ok(Self::build_error_response(status, b"Gateway Timeout"))
            }
        }
    }

    pub fn addr(&self) -> std::net::SocketAddr {
        self.listener
            .local_addr()
            .expect("Failed to get local address")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::BackendPool;
    use crate::config::BackendConfig;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::sync::oneshot;

    // --- Helpers ---

    fn setup_pool(configs: &[BackendConfig]) -> BackendPool {
        BackendPool::from_config(configs).expect("Failed to create BackendPool")
    }

    fn three_backends() -> Vec<BackendConfig> {
        (0..3)
            .map(|i| BackendConfig {
                addr: format!("0.0.0.{i}:8080"),
                weight: 1,
            })
            .collect()
    }

    /// Spawns the proxy and returns the base URL (http://127.0.0.1:PORT)
    async fn spawn_proxy(
        pool: Arc<BackendPool>,
        algorithm: Algorithm,
        proxy_timeout: Option<Duration>,
    ) -> String {
        let timeout = proxy_timeout.unwrap_or(Duration::from_secs(5));
        let proxy = Proxy::new(pool, algorithm, "127.0.0.1:0", timeout).await;
        let port = proxy.addr().port();
        tokio::spawn(async move {
            proxy.run().await;
        });
        format!("http://127.0.0.1:{port}")
    }

    /// A helper to spin up a dummy listener that captures the first request it receives
    async fn spawn_mock_backend() -> (String, oneshot::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("Failed to bind mock backend");
        let addr = listener
            .local_addr()
            .expect("Failed to read listener addr")
            .to_string();
        let (tx, rx) = oneshot::channel();

        tokio::spawn(async move {
            if let Ok((mut stream, _)) = listener.accept().await {
                let mut buf = [0; 2048];
                let n = stream
                    .read(&mut buf)
                    .await
                    .expect("Failed to read from stream");
                let request_str = String::from_utf8_lossy(&buf[..n]).to_string();

                let response = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello";
                stream
                    .write_all(response)
                    .await
                    .expect("Failed to write response");
                let _ = tx.send(request_str);
            }
        });

        (addr, rx)
    }

    //
    async fn spawn_mock_gated_backend(
        gate_rx: oneshot::Receiver<()>,
    ) -> (String, oneshot::Receiver<String>, oneshot::Receiver<()>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("Failed to bind mock backend");
        let addr = listener
            .local_addr()
            .expect("Failed to read listener addr")
            .to_string();
        let (tx, rx) = oneshot::channel();
        let (arrived_tx, arrived_rx) = oneshot::channel::<()>();

        tokio::spawn(async move {
            if let Ok((mut stream, _)) = listener.accept().await {
                let mut buf = [0; 2048];
                let n = stream
                    .read(&mut buf)
                    .await
                    .expect("Failed to read from stream");
                let request_str = String::from_utf8_lossy(&buf[..n]).to_string();

                let response = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello";

                // Signal that request has arrived
                let _ = arrived_tx.send(());

                // Wait for the gate signal before responding
                gate_rx.await.expect("Failed to read from stream");

                stream
                    .write_all(response)
                    .await
                    .expect("Failed to write response");
                let _ = tx.send(request_str);
            }
        });

        (addr, rx, arrived_rx)
    }

    // --- Refactored Tests ---

    #[tokio::test]
    async fn test_proxy_503_no_healthy_backends() {
        let pool = Arc::new(setup_pool(&three_backends()));
        // Manually kill all backends in the pool
        for i in 0..3 {
            pool.mark_unhealthy(i);
        }

        let url = spawn_proxy(pool, Algorithm::RoundRobin, None).await;
        let resp = reqwest::get(url).await.expect("Request failed");

        assert_eq!(resp.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn test_proxy_502_on_backend_error() {
        // Pool points to addresses that aren't listening
        let pool = Arc::new(setup_pool(&three_backends()));
        let url = spawn_proxy(pool, Algorithm::RoundRobin, None).await;

        let resp = reqwest::get(url).await.expect("Request failed");
        assert_eq!(resp.status(), reqwest::StatusCode::BAD_GATEWAY);
    }

    #[tokio::test]
    async fn test_proxy_forwards_request_and_payload() {
        let (addr, _) = spawn_mock_backend().await;
        let pool = Arc::new(setup_pool(&[BackendConfig { addr, weight: 1 }]));

        let url = spawn_proxy(pool, Algorithm::RoundRobin, None).await;
        let resp = reqwest::get(url).await.expect("Request failed");

        assert_eq!(resp.status(), reqwest::StatusCode::OK);
        assert_eq!(resp.text().await.expect("Failed to read response"), "hello");
    }

    #[tokio::test]
    async fn test_proxy_adds_required_headers() {
        let (addr, rx) = spawn_mock_backend().await;
        let pool = Arc::new(setup_pool(&[BackendConfig { addr, weight: 1 }]));
        let url = spawn_proxy(pool, Algorithm::RoundRobin, None).await;
        let _ = reqwest::get(url).await.expect("Request failed");

        let request_data = rx.await.expect("Mock backend did not receive request");
        let request_lc = request_data.to_lowercase();

        assert!(
            request_lc.contains("x-forwarded-for:"),
            "Missing X-Forwarded-For"
        );
        assert!(
            request_lc.contains("x-forwarded-host:"),
            "Missing X-Forwarded-Host"
        );
        assert!(
            request_lc.contains("x-forwarded-proto:"),
            "Missing X-Forwarded-Proto"
        );
        assert!(
            request_lc.contains("x-forwarded-proto: http"),
            "X-Forwarded-Proto should be 'http'"
        );
    }

    #[tokio::test]
    async fn test_proxy_strips_hop_by_hop() {
        let (addr, rx) = spawn_mock_backend().await;
        let pool = Arc::new(setup_pool(&[BackendConfig { addr, weight: 1 }]));

        let url = spawn_proxy(pool, Algorithm::RoundRobin, None).await;
        let client = reqwest::Client::new();
        let _ = client
            .get(url)
            .header("Connection", "keep-alive")
            .header("Keep-Alive", "timeout=5")
            .header(
                "Proxy-Authenticate",
                "Basic realm=\"Access to the staging site\"",
            )
            .header("Proxy-Authorization", "Basic dXNlcjpwYXNz")
            .header("TE", "trailers, deflate")
            .header("Trailers", "Expires")
            .header("Transfer-Encoding", "chunked")
            .header("Upgrade", "websocket")
            .send()
            .await
            .expect("Request failed");

        let request_data = rx.await.expect("Mock backend did not receive request");
        let request_lc = request_data.to_lowercase();

        assert!(
            !request_lc.contains("connection:"),
            "Hop-by-hop header 'Connection' was not stripped"
        );
        assert!(
            !request_lc.contains("keep-alive:"),
            "Hop-by-hop header 'Keep-Alive' was not stripped"
        );
        assert!(
            !request_lc.contains("proxy-authenticate:"),
            "Hop-by-hop header 'Proxy-Authenticate' was not stripped"
        );
        assert!(
            !request_lc.contains("proxy-authorization:"),
            "Hop-by-hop header 'Proxy-Authorization' was not stripped"
        );
        assert!(
            !request_lc.contains("te:"),
            "Hop-by-hop header 'TE' was not stripped"
        );
        assert!(
            !request_lc.contains("trailers:"),
            "Hop-by-hop header 'Trailers' was not stripped"
        );
        assert!(
            !request_lc.contains("transfer-encoding:"),
            "Hop-by-hop header 'Transfer-Encoding' was not stripped"
        );
        assert!(
            !request_lc.contains("upgrade:"),
            "Hop-by-hop header 'Upgrade' was not stripped"
        );
    }

    #[tokio::test]
    async fn test_connection_count_increments() {
        let (gate_tx, gate_rx) = oneshot::channel();
        let (addr, _rx, arrived_rx) = spawn_mock_gated_backend(gate_rx).await;
        let pool = Arc::new(setup_pool(&[BackendConfig { addr, weight: 1 }]));
        let url = spawn_proxy(Arc::clone(&pool), Algorithm::RoundRobin, None).await;
        let request_handle =
            tokio::spawn(async move { reqwest::get(url).await.expect("Request failed") });

        arrived_rx.await.expect("Request never arrived at backend");
        assert_eq!(pool.all_backends()[0].get_active_connections(), 1);

        gate_tx.send(()).expect("Failed to open gate");
        let _resp = request_handle.await.expect("Task panicked");
        assert_eq!(pool.all_backends()[0].get_active_connections(), 0);
    }

    #[tokio::test]
    async fn test_proxy_504_on_timeout() {
        let (_gate_tx, gate_rx) = oneshot::channel();
        let (addr, _rx, _arrived_rx) = spawn_mock_gated_backend(gate_rx).await;
        let pool = Arc::new(setup_pool(&[BackendConfig { addr, weight: 1 }]));
        let url = spawn_proxy(pool, Algorithm::RoundRobin, Some(Duration::from_micros(50))).await;
        let request_handle =
            tokio::spawn(async move { reqwest::get(url).await.expect("Request failed") });

        let resp = request_handle.await.expect("Task panicked");
        assert_eq!(resp.status(), reqwest::StatusCode::GATEWAY_TIMEOUT);
    }
}
