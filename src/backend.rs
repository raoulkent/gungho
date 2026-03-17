use crate::config::{BackendConfig, Config};
use std::net::{AddrParseError, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize};

struct Backend {
    addr: SocketAddr,
    weight: u32,
    healthy: AtomicBool,
    active_connections: AtomicUsize,
}

impl Backend {
    pub fn from_config(config: &BackendConfig) -> Result<Self, AddrParseError> {
        Ok(Backend {
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

#[derive(Debug, thiserror::Error)]
pub enum BackendPoolError {
    #[error("No backends configured")]
    NoBackends,
    #[error("Invalid backend address: {0}")]
    AddrParseError(#[from] AddrParseError),
}

impl BackendPoolError {
    pub fn from_addr_parse_error(err: AddrParseError) -> Self {
        BackendPoolError::AddrParseError(err)
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

        Ok(BackendPool {
            backends: backends.into_iter().map(Arc::new).collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn test_pool_from_config() {}

    #[test]
    fn test_healthy_filter() {}

    #[test]
    fn test_mark_unhealthy() {}

    #[test]
    fn test_mark_healthy_recovery() {}

    #[test]
    fn test_connection_counting() {}

    #[test]
    fn test_empty_config_errors() {}
}
