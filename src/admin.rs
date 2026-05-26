use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::server::conn::http1::Builder;
use hyper::{Request, Response, StatusCode};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use crate::backend::BackendPool;
use crate::metrics::GunghoMetrics;

struct Admin {
    metrics: Arc<GunghoMetrics>,
    pool: Arc<BackendPool>,
    listener: TcpListener,
    cancellation_token: CancellationToken,
}

impl Admin {
    async fn new(
        metrics: Arc<GunghoMetrics>,
        pool: Arc<BackendPool>,
        listener_addr: SocketAddr,
        cancellation_token: CancellationToken,
    ) -> Result<Self, std::io::Error> {
        let listener: TcpListener = TcpListener::bind(listener_addr).await?;

        Ok(Self {
            metrics,
            pool,
            listener,
            cancellation_token,
        })
    }

    fn local_addr(&self) -> SocketAddr {
        self.listener
            .local_addr()
            .expect("Could not get local_addr")
    }

    async fn run(self) {
        let http: Arc<Builder> = Arc::new(Builder::new());
        let this = Arc::new(self);

        loop {
            tokio::select! {
                () = this.cancellation_token.cancelled() => break,
                result = this.listener.accept() => {
                    let (stream, _client_addr) = result.expect("Failed to accept connection");
                    let this = Arc::clone(&this);
                    let http = Arc::clone(&http);
                    tokio::spawn(async move {
                        let connection = http.serve_connection(
                            hyper_util::rt::TokioIo::new(stream),
                            hyper::service::service_fn(move |req| {
                                let this = Arc::clone(&this);
                                async move { Ok::<_, Infallible>(this.handle_request(&req)) }
                            }),
                        );

                        if let Err(e) = connection.await {
                            eprintln!("Connection error: {e}");
                        }
                    });
                },
            }
        }
    }

    fn handle_request(self: Arc<Self>, request: &Request<Incoming>) -> Response<Full<Bytes>> {
        let path = request.uri().path();

        match path {
            "/metrics" => Self::handle_metrics(self),
            "/livez" => Self::handle_livez(),
            "/readyz" => Self::handle_readyz(self),
            _ => Self::handle_default(),
        }
    }

    fn handle_metrics(self: Arc<Self>) -> Response<Full<Bytes>> {
        let metrics = self.metrics.encode_metrics();

        Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "text/plain; version=0.0.4; charset=utf-8")
            .body(Full::new(Bytes::from(metrics)))
            .expect("Failed to build response")
    }

    fn handle_livez() -> Response<Full<Bytes>> {
        Response::builder()
            .status(StatusCode::OK)
            .body(Full::new(Bytes::from_static(br#"{"status": "ok"}"#)))
            .expect("Failed to build response")
    }

    fn handle_readyz(self: Arc<Self>) -> Response<Full<Bytes>> {
        let healthy = self.pool.healthy_backends();
        let healthy_count = healthy.len();

        let (body, status) = if healthy.is_empty() {
            (
                r#"{"status": "not ready", "healthy_backends": 0}"#.into(),
                StatusCode::SERVICE_UNAVAILABLE,
            )
        } else {
            (
                format!(r#"{{"status": "ok", "healthy_backends": {healthy_count}}}"#),
                StatusCode::OK,
            )
        };

        Response::builder()
            .status(status)
            .body(Full::new(Bytes::from(body)))
            .expect("could not build response")
    }

    fn handle_default() -> Response<Full<Bytes>> {
        Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Full::new(Bytes::from_static(
                br#"{"status": "could not be found"}"#,
            )))
            .expect("Failed to build response")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{Backend, BackendPool};

    async fn spawn_admin_server(pool: Arc<BackendPool>) -> (CancellationToken, SocketAddr) {
        let metrics = Arc::from(GunghoMetrics::new());
        let listener_addr: SocketAddr = "127.0.0.1:0".parse().expect("Failed to parse address");
        let cancellation_token = CancellationToken::new();

        let admin = Admin::new(metrics, pool, listener_addr, cancellation_token.clone())
            .await
            .expect("Could not create new Admin");
        let addr = admin.local_addr();

        tokio::spawn(admin.run());

        (cancellation_token, addr)
    }

    #[tokio::test]
    async fn test_livez_always_200() {
        let pool = Arc::new(BackendPool::default());
        let (_cancellation_token, addr) = spawn_admin_server(pool).await;

        let resp = reqwest::get(format!("http://{addr}/livez"))
            .await
            .expect("Request failed");

        assert_eq!(resp.status(), reqwest::StatusCode::OK);
        assert_eq!(
            resp.text().await.expect("Failed to read response"),
            r#"{"status": "ok"}"#
        );
    }

    #[tokio::test]
    async fn test_readyz_healthy() {
        let addr: SocketAddr = "127.0.0.1:0".parse().expect("Failed to parse address");
        let backend: Arc<Backend> = Arc::new(Backend::new(addr));
        let backends = vec![backend];
        let pool = Arc::new(BackendPool::new(backends));
        let (_cancellation_token, addr) = spawn_admin_server(pool).await;

        let resp = reqwest::get(format!("http://{addr}/readyz"))
            .await
            .expect("Could not get readyz response");

        assert_eq!(resp.status(), reqwest::StatusCode::OK);
        assert_eq!(
            resp.text().await.expect("Failed to read response"),
            r#"{"status": "ok", "healthy_backends": 1}"#
        );
    }

    #[tokio::test]
    async fn test_readyz_all_unhealthy() {
        let addr: SocketAddr = "127.0.0.1:0".parse().expect("Failed to parse address");
        let backend: Arc<Backend> = Arc::new(Backend::new(addr));
        backend.set_health(false);
        let backends = vec![backend];
        let pool = Arc::new(BackendPool::new(backends));
        let (_cancellation_token, addr) = spawn_admin_server(pool).await;

        let resp = reqwest::get(format!("http://{addr}/readyz"))
            .await
            .expect("Could not get readyz response");

        assert_eq!(resp.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            resp.text().await.expect("Failed to read response"),
            r#"{"status": "not ready", "healthy_backends": 0}"#
        );
    }

    #[tokio::test]
    async fn test_metrics_endpoint() {
        let pool = Arc::new(BackendPool::default());
        let (_cancellation_token, addr) = spawn_admin_server(pool).await;

        let resp = reqwest::get(format!("http://{addr}/metrics"))
            .await
            .expect("Request failed");
        let headers = resp.headers().clone();
        let body = resp.text().await.expect("Failed to read response");

        assert_eq!(
            headers
                .get("content-type")
                .expect("Could not get content type"),
            "text/plain; version=0.0.4; charset=utf-8"
        );
        assert!(body.contains("# HELP"));
        assert!(body.contains("# TYPE"));
        assert!(body.contains("gungho_active_connections 0"));
        assert!(body.contains("gungho_backends_total 0"));
    }

    #[tokio::test]
    async fn test_unknown_path_404() {
        let pool = Arc::new(BackendPool::default());
        let (_cancellation_token, addr) = spawn_admin_server(pool).await;

        let resp = reqwest::get(format!("http://{addr}/foo/bar/spam-eggs"))
            .await
            .expect("Request failed");

        assert_eq!(resp.status(), reqwest::StatusCode::NOT_FOUND);
        assert_eq!(
            resp.text().await.expect("Failed to read response"),
            r#"{"status": "could not be found"}"#
        );
    }
}
