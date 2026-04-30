use bytes::Bytes;
use http_body_util::{Either, Full};

use hyper::body::Incoming;
use hyper::{Request, Response};
use tokio::net::TcpListener;

use crate::backend::BackendPool;
use crate::config::Algorithm;
use crate::lb;
use crate::lb::LoadBalancingStrategy;

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

struct Proxy {
    listener: TcpListener,
    backend_pool: Arc<BackendPool>,
    strategy: Arc<dyn LoadBalancingStrategy>,
}

impl Proxy {
    pub async fn new(pool: BackendPool, algorithm: Algorithm, addr: &str) -> Self {
        let backend_pool = Arc::new(pool);
        let strategy = Arc::from(lb::create_strategy(&algorithm));

        let listener = TcpListener::bind(addr)
            .await
            .expect("Failed to bind to address");
        Self {
            listener,
            backend_pool,
            strategy,
        }
    }

    pub async fn run(self) {
        let http = hyper::server::conn::http1::Builder::new();

        loop {
            let (stream, client_addr) = self
                .listener
                .accept()
                .await
                .expect("Failed to accept connection");

            let pool = Arc::clone(&self.backend_pool);
            let strategy = Arc::clone(&self.strategy);

            let connection = http.serve_connection(
                hyper_util::rt::TokioIo::new(stream),
                hyper::service::service_fn(move |req| {
                    let pool = Arc::clone(&pool);
                    let strategy = Arc::clone(&strategy);
                    async move { Self::handle_request(req, client_addr, pool, strategy).await }
                }),
            );

            tokio::spawn(async move {
                if let Err(e) = connection.await {
                    eprintln!("Connection error: {e}");
                }
            });
        }
    }

    async fn handle_request(
        mut req: Request<Incoming>,
        client_addr: SocketAddr,
        pool: Arc<BackendPool>,
        strategy: Arc<dyn LoadBalancingStrategy>,
    ) -> Result<Response<Either<Incoming, Full<Bytes>>>, Infallible> {
        let healthy = pool.healthy_backends();

        let Some(index) = strategy.select(&healthy, Some(&client_addr)) else {
            // no backend selected → 503
            return Ok(Response::builder()
                .status(503)
                .body(Either::Right(Full::new(Bytes::from_static(
                    b"No healthy backends available",
                ))))
                .expect("Failed to build response"));
        };

        let backend = &healthy[index];
        let backend_addr = backend.get_addr();

        let client =
            hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
                .build_http();

        *req.uri_mut() = format!("http://{backend_addr}{}", req.uri().path())
            .parse()
            .expect("Failed to parse URI");

        // Add X-Forwarded-For and X-Forwarded-Host headers
        req.headers_mut().insert(
            "X-Forwarded-For",
            client_addr.ip().to_string().parse().expect("Invalid IP"),
        );
        req.headers_mut().insert(
            "X-Forwarded-Host",
            client_addr.to_string().parse().expect("Invalid Host"),
        );

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
            req.headers_mut().remove(*header);
        }

        let response = client.request(req).await;

        #[allow(clippy::option_if_let_else)]
        match response {
            Ok(resp) => Ok(resp.map(Either::Left)),
            Err(_) => Ok(Response::builder()
                .status(502)
                .body(Either::Right(Full::new(Bytes::from_static(
                    b"Backend error",
                ))))
                .expect("Failed to build response")),
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
    async fn spawn_proxy(pool: BackendPool, algorithm: Algorithm) -> String {
        let proxy = Proxy::new(pool, algorithm, "127.0.0.1:0").await;
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

    // --- Refactored Tests ---

    #[tokio::test]
    async fn test_proxy_503_no_healthy_backends() {
        let pool = setup_pool(&three_backends());
        // Manually kill all backends in the pool
        for i in 0..3 {
            pool.mark_unhealthy(i);
        }

        let url = spawn_proxy(pool, Algorithm::RoundRobin).await;
        let resp = reqwest::get(url).await.expect("Request failed");

        assert_eq!(resp.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn test_proxy_502_on_backend_error() {
        // Pool points to addresses that aren't listening
        let pool = setup_pool(&three_backends());
        let url = spawn_proxy(pool, Algorithm::RoundRobin).await;

        let resp = reqwest::get(url).await.expect("Request failed");
        assert_eq!(resp.status(), reqwest::StatusCode::BAD_GATEWAY);
    }

    #[tokio::test]
    async fn test_proxy_forwards_request_and_payload() {
        let (addr, _) = spawn_mock_backend().await;
        let pool = setup_pool(&[BackendConfig { addr, weight: 1 }]);

        let url = spawn_proxy(pool, Algorithm::RoundRobin).await;
        let resp = reqwest::get(url).await.expect("Request failed");

        assert_eq!(resp.status(), reqwest::StatusCode::OK);
        assert_eq!(resp.text().await.expect("Failed to read response"), "hello");
    }

    #[tokio::test]
    async fn test_proxy_adds_required_headers() {
        let (addr, rx) = spawn_mock_backend().await;
        let pool = setup_pool(&[BackendConfig { addr, weight: 1 }]);

        let url = spawn_proxy(pool, Algorithm::RoundRobin).await;
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
    }

    #[tokio::test]
    async fn test_proxy_strips_hop_by_hop() {
        let (addr, rx) = spawn_mock_backend().await;
        let pool = setup_pool(&[BackendConfig { addr, weight: 1 }]);

        let url = spawn_proxy(pool, Algorithm::RoundRobin).await;
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
}
