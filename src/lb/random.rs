use std::net::SocketAddr;
use std::sync::Arc;

use crate::backend::Backend;
use crate::lb::LoadBalancingStrategy;

#[derive(Debug)]
pub struct Random;

impl Random {
    pub const fn new() -> Self {
        Self
    }
}

impl LoadBalancingStrategy for Random {
    fn select(
        &self,
        backends: &[Arc<Backend>],
        _client_addr: Option<&SocketAddr>,
    ) -> Option<usize> {
        if backends.is_empty() {
            return None;
        }

        let index = rand::random_range(0..backends.len());

        Some(index)
    }

    fn algorithm(&self) -> &crate::config::Algorithm {
        &crate::config::Algorithm::Random
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::BackendPool;
    use crate::config::BackendConfig;

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

    #[test]
    fn test_returns_valid_index() {
        let strategy = Random::new();
        let pool = setup_pool(&three_backends());

        for _ in 0..=1000 {
            assert!(
                matches!(strategy.select(&pool.healthy_backends(), None), Some(index) if index < pool.healthy_backends().len())
            );
        }
    }

    #[test]
    fn test_empty_returns_none() {
        let strategy = Random::new();
        let empty: Vec<Arc<Backend>> = Vec::new();

        assert!(strategy.select(&empty, None).is_none());
    }
}
