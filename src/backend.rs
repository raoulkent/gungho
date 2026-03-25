use crate::config::{BackendConfig, Config};
use std::net::{AddrParseError, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::atomic::{AtomicBool, AtomicUsize};

struct Backend {
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
}

struct BackendPool {
    backends: Vec<Arc<Backend>>,
}

#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum BackendPoolError {
    #[error("No backends configured")]
    NoBackends,
    #[error("Invalid backend address: {0}")]
    AddrParseError(#[from] AddrParseError),
}

impl BackendPoolError {
    pub const fn from_addr_parse_error(err: AddrParseError) -> Self {
        Self::AddrParseError(err)
    }
}

impl BackendPool {
    pub fn from_config(config: &Config) -> Result<Self, BackendPoolError> {
        if config.backends.is_empty() {
            return Err(BackendPoolError::NoBackends);
        }

        let backends = config
            .backends
            .iter()
            .map(Backend::from_config)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self {
            backends: backends.into_iter().map(Arc::new).collect(),
        })
    }

    fn healthy_backends(&self) -> Vec<Arc<Backend>> {
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

        let config = Config {
            backends: backends.to_vec(),
            ..Config::default()
        };

        let pool = BackendPool::from_config(&config).expect("BackendPool ");

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

        let config = Config {
            backends: backends.to_vec(),
            ..Config::default()
        };

        let pool =
            BackendPool::from_config(&config).expect("could not read backendpool from config");

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

        let config = Config {
            backends: backends.to_vec(),
            ..Config::default()
        };

        let pool =
            BackendPool::from_config(&config).expect("Could not read backendpool from config");

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

        let config = Config {
            backends: backends.to_vec(),
            ..Config::default()
        };

        let pool =
            BackendPool::from_config(&config).expect("Could not read backendpool from config");

        pool.mark_unhealthy(1);

        assert_eq!(pool.backends[1].healthy.load(Ordering::SeqCst), false);

        pool.mark_healthy(1);

        assert_eq!(pool.backends[1].healthy.load(Ordering::SeqCst), true);
    }

    #[test]
    fn test_connection_counting() {}

    #[test]
    fn test_empty_config_errors() {
        let config = Config::default();

        let pool = BackendPool::from_config(&config);

        assert_eq!(pool.err(), Some(BackendPoolError::NoBackends));
    }
}
