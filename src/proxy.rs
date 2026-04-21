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

        req.headers_mut().insert(
            "X-Forwarded-For",
            client_addr.ip().to_string().parse().expect("Invalid IP"),
        );

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

    use reqwest::get;

    fn setup_pool(configs: &[BackendConfig]) -> BackendPool {
        BackendPool::from_config(configs).expect("Failed to create BackendPool")
    }

    fn three_backends() -> Vec<BackendConfig> {
        vec![
            BackendConfig {
                addr: "0.0.0.0:8080".into(),
                weight: 1,
            },
            BackendConfig {
                addr: "0.0.0.1:8080".into(),
                weight: 1,
            },
            BackendConfig {
                addr: "0.0.0.2:8080".into(),
                weight: 1,
            },
        ]
    }

    #[tokio::test]
    async fn test_proxy_503_no_healthy_backends() {
        let backends = three_backends();
        let pool = setup_pool(&backends);
        pool.mark_unhealthy(0);
        pool.mark_unhealthy(1);
        pool.mark_unhealthy(2);
        let algorithm = Algorithm::RoundRobin; // doesn't matter since all backends are unhealthy

        let proxy = Proxy::new(pool, algorithm, "127.0.0.1:0").await;
        let port = proxy.addr().port();

        tokio::spawn(async move {
            proxy.run().await; // runs forever in the background
        });

        let resp = get(format!("http://127.0.0.1:{port}"))
            .await
            .expect("Could not await response");

        assert_eq!(resp.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn test_proxy_502_on_backend_error() {
        let backends = three_backends();
        let pool = setup_pool(&backends);
        let algorithm = Algorithm::RoundRobin;

        let proxy = Proxy::new(pool, algorithm, "127.0.0.1:0").await;
        let port = proxy.addr().port();

        tokio::spawn(async move {
            proxy.run().await; // runs forever in the background
        });

        let resp = get(format!("http://127.0.0.1:{port}"))
            .await
            .expect("Could not await response");

        assert_eq!(resp.status(), reqwest::StatusCode::BAD_GATEWAY);
    }

    #[tokio::test]
    async fn test_proxy_forwards_request() {
        let mock_backend = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("Failed to bind mock backend");
        let mock_addr = mock_backend.local_addr().expect("Failed to get mock addr");
        tokio::spawn(async move {
            let (mut stream, _) = mock_backend.accept().await.expect("Failed to accept");
            tokio::io::AsyncWriteExt::write_all(
                &mut stream,
                b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello",
            )
            .await
            .expect("Failed to write response");
        });

        let backends = vec![BackendConfig {
            addr: mock_addr.to_string(),
            weight: 1,
        }];
        let pool = setup_pool(&backends);
        let algorithm = Algorithm::RoundRobin;

        let proxy = Proxy::new(pool, algorithm, "127.0.0.1:0").await;
        let port = proxy.addr().port();
        tokio::spawn(async move {
            proxy.run().await;
        });

        let resp = reqwest::get(format!("http://127.0.0.1:{port}"))
            .await
            .expect("Request failed");
        assert_eq!(resp.status(), reqwest::StatusCode::OK);
        assert_eq!(resp.text().await.expect("Failed to read body"), "hello");
    }

    #[tokio::test]
    async fn test_proxy_adds_xforwarded_for() {
        let mock_backend = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("Failed to bind mock backend");
        let mock_addr = mock_backend.local_addr().expect("Failed to get mock addr");

        let (tx, rx) = tokio::sync::oneshot::channel::<String>();
        tokio::spawn(async move {
            let (mut stream, _) = mock_backend.accept().await.expect("Failed to accept");

            let mut buff = [0; 1024];
            let n = tokio::io::AsyncReadExt::read(&mut stream, &mut buff)
                .await
                .expect("Failed to read request");

            tx.send(String::from_utf8_lossy(&buff[..n]).to_string())
                .expect("Failed to send request data");

            tokio::io::AsyncWriteExt::write_all(
                &mut stream,
                b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello",
            )
            .await
            .expect("Failed to write response");
        });

        let backends = vec![BackendConfig {
            addr: mock_addr.to_string(),
            weight: 1,
        }];
        let pool = setup_pool(&backends);
        let algorithm = Algorithm::RoundRobin;

        let proxy = Proxy::new(pool, algorithm, "127.0.0.1:0").await;
        let port = proxy.addr().port();
        tokio::spawn(async move {
            proxy.run().await;
        });

        let resp = reqwest::get(format!("http://127.0.0.1:{port}"))
            .await
            .expect("Request failed");
        assert_eq!(resp.status(), reqwest::StatusCode::OK);
        let rx_data = rx.await.expect("Failed to receive request data");
        eprintln!("Received request:\n{rx_data}");
        assert!(rx_data.contains("x-forwarded-for:"));
    }
}
