use crate::config::BackendConfig;
use std::net::{AddrParseError, SocketAddr};
use std::sync::atomic::Ordering;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::{Arc, RwLock};

pub struct Backend {
    addr: SocketAddr,
    weight: u32,
    healthy: AtomicBool,
    active_connections: AtomicUsize,
}

impl Default for Backend {
    fn default() -> Self {
        Self::new(
            "127.0.0.1:0"
                .parse()
                .expect("Could not parse default SocketAddr"),
        )
    }
}

impl Backend {
    pub(crate) const fn new(addr: SocketAddr) -> Self {
        Self {
            addr,
            weight: 1,
            healthy: AtomicBool::new(true),
            active_connections: AtomicUsize::new(0),
        }
    }

    pub fn from_config(config: &BackendConfig) -> Result<Self, AddrParseError> {
        Ok(Self {
            addr: config.addr.parse::<std::net::SocketAddr>()?,
            weight: config.weight,
            healthy: AtomicBool::new(true),
            active_connections: AtomicUsize::new(0),
        })
    }

    pub const fn get_addr(&self) -> SocketAddr {
        self.addr
    }

    pub const fn get_weight(&self) -> u32 {
        self.weight
    }

    pub fn get_health(&self) -> bool {
        self.healthy.load(Ordering::Acquire)
    }

    pub fn set_health(&self, healthy: bool) {
        self.healthy.store(healthy, Ordering::SeqCst);
    }

    pub fn get_active_connections(&self) -> usize {
        self.active_connections.load(Ordering::Relaxed)
    }

    pub fn increment_connections(&self) {
        self.active_connections.fetch_add(1, Ordering::Relaxed);
    }

    pub fn decrement_connections(&self) {
        self.active_connections.fetch_sub(1, Ordering::Relaxed);
    }
}

type HealthyCache = Arc<Vec<Arc<Backend>>>;

pub struct BackendPool {
    backends: Vec<Arc<Backend>>,
    healthy_cache: RwLock<HealthyCache>,
}

impl Default for BackendPool {
    fn default() -> Self {
        Self {
            backends: vec![],
            healthy_cache: RwLock::new(Arc::new(vec![])),
        }
    }
}

#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum BackendPoolError {
    #[error("No backends configured")]
    NoBackends,
    #[error("Invalid backend address: {0}")]
    AddrParseError(#[from] AddrParseError),
}

impl BackendPool {
    pub(crate) fn new(backends: Vec<Arc<Backend>>) -> Self {
        let healthy_cache = RwLock::new(Arc::new(
            backends.iter().filter(|b| b.get_health()).cloned().collect(),
        ));
        Self {
            backends,
            healthy_cache,
        }
    }

    pub fn from_config(backends: &[BackendConfig]) -> Result<Self, BackendPoolError> {
        if backends.is_empty() {
            return Err(BackendPoolError::NoBackends);
        }

        let backends = backends
            .iter()
            .map(Backend::from_config)
            .collect::<Result<Vec<_>, _>>()?;

        let backends: Vec<Arc<Backend>> = backends.into_iter().map(Arc::new).collect();
        let healthy_cache = RwLock::new(Arc::new(backends.clone()));

        Ok(Self {
            backends,
            healthy_cache,
        })
    }

    pub fn all_backends(&self) -> &[Arc<Backend>] {
        &self.backends
    }

    pub fn get_health_by_addr(&self, addr: SocketAddr) -> Option<bool> {
        self.backends
            .iter()
            .find(|b| b.addr == addr)
            .map(|b| b.healthy.load(Ordering::Acquire))
    }

    pub fn healthy_backends(&self) -> Arc<Vec<Arc<Backend>>> {
        self.healthy_cache
            .read()
            .expect("healthy_cache lock poisoned")
            .clone()
    }

    fn rebuild_healthy_cache(&self) {
        let healthy: Vec<Arc<Backend>> = self
            .backends
            .iter()
            .filter(|b| b.get_health())
            .cloned()
            .collect();
        *self
            .healthy_cache
            .write()
            .expect("healthy_cache lock poisoned") = Arc::new(healthy);
    }

    pub(crate) fn mark_healthy(&self, index: usize) -> Option<()> {
        self.backends
            .get(index)?
            .healthy
            .store(true, Ordering::Release);
        self.rebuild_healthy_cache();

        Some(())
    }

    pub(crate) fn mark_unhealthy(&self, index: usize) -> Option<()> {
        self.backends
            .get(index)?
            .healthy
            .store(false, Ordering::Release);
        self.rebuild_healthy_cache();

        Some(())
    }

    pub(crate) fn mark_healthy_by_addr(&self, addr: SocketAddr) -> Option<()> {
        self.backends
            .iter()
            .find(|b| b.addr == addr)?
            .healthy
            .store(true, Ordering::Release);
        self.rebuild_healthy_cache();

        Some(())
    }

    pub(crate) fn mark_unhealthy_by_addr(&self, addr: SocketAddr) -> Option<()> {
        self.backends
            .iter()
            .find(|b| b.addr == addr)?
            .healthy
            .store(false, Ordering::Release);
        self.rebuild_healthy_cache();

        Some(())
    }

    fn increment_connections(&self, index: usize) -> Option<()> {
        self.backends.get(index)?.increment_connections();

        Some(())
    }

    fn decrement_connections(&self, index: usize) -> Option<()> {
        self.backends.get(index)?.decrement_connections();

        Some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn test_pool_from_config() {
        let backends = [
            BackendConfig {
                addr: "0.0.0.0:8080".to_string(),
                weight: 1,
            },
            BackendConfig {
                addr: "0.0.0.1:8080".to_string(),
                weight: 2,
            },
        ];

        let pool = BackendPool::from_config(&backends).expect("BackendPool ");

        assert_eq!(pool.backends.len(), 2);
        assert_eq!(
            pool.backends[0].addr,
            "0.0.0.0:8080"
                .parse::<SocketAddr>()
                .expect("Could not parse address")
        );
    }

    #[test]
    fn test_healthy_filter() {
        let backends = [
            BackendConfig {
                addr: "0.0.0.0:8080".to_string(),
                weight: 1,
            },
            BackendConfig {
                addr: "0.0.0.1:8080".to_string(),
                weight: 2,
            },
        ];

        let pool =
            BackendPool::from_config(&backends).expect("could not read backendpool from config");

        assert_eq!(pool.healthy_backends().len(), 2);
    }

    #[test]
    fn test_mark_unhealthy() {
        let backends = [
            BackendConfig {
                addr: "0.0.0.0:8080".to_string(),
                weight: 1,
            },
            BackendConfig {
                addr: "0.0.0.1:8080".to_string(),
                weight: 2,
            },
        ];

        let pool =
            BackendPool::from_config(&backends).expect("Could not read backendpool from config");

        pool.mark_unhealthy(0);

        assert_eq!(pool.healthy_backends().len(), 1);
    }

    #[test]
    fn test_mark_healthy_recovery() {
        let backends = [
            BackendConfig {
                addr: "0.0.0.0:8080".to_string(),
                weight: 1,
            },
            BackendConfig {
                addr: "0.0.0.1:8080".to_string(),
                weight: 2,
            },
        ];

        let pool =
            BackendPool::from_config(&backends).expect("Could not read backendpool from config");

        pool.mark_unhealthy(1);

        assert_eq!(pool.backends[1].healthy.load(Ordering::Acquire), false);

        pool.mark_healthy(1);

        assert_eq!(pool.backends[1].healthy.load(Ordering::Acquire), true);
    }

    #[test]
    fn test_connection_counting() {
        let backends = [
            BackendConfig {
                addr: "0.0.0.0:8080".to_string(),
                weight: 1,
            },
            BackendConfig {
                addr: "0.0.0.1:8080".to_string(),
                weight: 2,
            },
        ];

        let pool = BackendPool::from_config(&backends).expect("BackendPool ");

        assert_eq!(
            pool.backends[0].active_connections.load(Ordering::Relaxed),
            0
        );
        assert_eq!(
            pool.backends[1].active_connections.load(Ordering::Relaxed),
            0,
        );

        for _ in 1..=13 {
            pool.increment_connections(1);
        }

        assert_eq!(
            pool.backends[1].active_connections.load(Ordering::Relaxed),
            13
        );

        for _ in 1..=7 {
            pool.decrement_connections(1);
        }

        assert_eq!(
            pool.backends[1].active_connections.load(Ordering::Relaxed),
            6
        );
    }

    #[test]
    fn test_empty_config_errors() {
        let backends: &[BackendConfig] = &[];

        let pool = BackendPool::from_config(backends);

        assert_eq!(pool.err(), Some(BackendPoolError::NoBackends));
    }
}
