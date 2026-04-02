use crate::config::BackendConfig;
use std::net::{AddrParseError, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::atomic::{AtomicBool, AtomicUsize};

pub struct Backend {
    addr: SocketAddr,
    weight: u32,
    healthy: AtomicBool,
    active_connections: AtomicUsize,
}

impl Backend {
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

    pub const fn set_weight(&mut self, weight: u32) {
        self.weight = weight;
    }

    pub fn get_health(&self) -> bool {
        self.healthy.load(Ordering::SeqCst)
    }

    pub fn get_active_connections(&self) -> usize {
        self.active_connections.load(Ordering::SeqCst)
    }

    pub fn increment_connections(&self) {
        self.active_connections.fetch_add(1, Ordering::SeqCst);
    }

    pub fn decrement_connections(&self) {
        self.active_connections.fetch_sub(1, Ordering::SeqCst);
    }
}

pub struct BackendPool {
    backends: Vec<Arc<Backend>>,
}

#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum BackendPoolError {
    #[error("No backends configured")]
    NoBackends,
    #[error("Invalid backend address: {0}")]
    AddrParseError(#[from] AddrParseError),
}

impl BackendPool {
    pub fn from_config(backends: &[BackendConfig]) -> Result<Self, BackendPoolError> {
        if backends.is_empty() {
            return Err(BackendPoolError::NoBackends);
        }

        let backends = backends
            .iter()
            .map(Backend::from_config)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self {
            backends: backends.into_iter().map(Arc::new).collect(),
        })
    }

    pub fn all_backends(&self) -> &[Arc<Backend>] {
        &self.backends
    }

    pub fn healthy_backends(&self) -> Vec<Arc<Backend>> {
        self.backends
            .iter()
            .filter(|b| b.healthy.load(Ordering::SeqCst))
            .cloned()
            .collect()
    }

    fn mark_healthy(&self, index: usize) -> Option<()> {
        self.backends
            .get(index)?
            .healthy
            .store(true, Ordering::SeqCst);

        Some(())
    }

    fn mark_unhealthy(&self, index: usize) -> Option<()> {
        self.backends
            .get(index)?
            .healthy
            .store(false, Ordering::SeqCst);

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

        assert_eq!(pool.backends[1].healthy.load(Ordering::SeqCst), false);

        pool.mark_healthy(1);

        assert_eq!(pool.backends[1].healthy.load(Ordering::SeqCst), true);
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
            pool.backends[0].active_connections.load(Ordering::SeqCst),
            0
        );
        assert_eq!(
            pool.backends[1].active_connections.load(Ordering::SeqCst),
            0,
        );

        for _ in 1..=13 {
            pool.increment_connections(1);
        }

        assert_eq!(
            pool.backends[1].active_connections.load(Ordering::SeqCst),
            13
        );

        for _ in 1..=7 {
            pool.decrement_connections(1);
        }

        assert_eq!(
            pool.backends[1].active_connections.load(Ordering::SeqCst),
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
