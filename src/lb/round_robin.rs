use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use crate::backend::Backend;
use crate::lb::LoadBalancingStrategy;

#[derive(Debug)]
pub struct RoundRobin {
    counter: AtomicUsize,
}

impl RoundRobin {
    pub const fn new() -> Self {
        Self {
            counter: AtomicUsize::new(0),
        }
    }
}

impl LoadBalancingStrategy for RoundRobin {
    fn select(
        &self,
        backends: &[Arc<Backend>],
        _client_addr: Option<&SocketAddr>,
    ) -> Option<usize> {
        if backends.is_empty() {
            return None;
        }
        let index = self.counter.fetch_add(1, Ordering::SeqCst) % backends.len();
        Some(index)
    }

    fn algorithm(&self) -> &crate::config::Algorithm {
        &crate::config::Algorithm::RoundRobin
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
    fn test_even_distribution() {
        let pool = setup_pool(&three_backends());
        let strategy = RoundRobin::new();
        let healthy = pool.healthy_backends();

        for i in 0..300 {
            let selected = strategy
                .select(&healthy, None)
                .expect("Should select a backend");

            assert_eq!(selected, i % healthy.len());
        }
    }

    #[test]
    fn test_wraps_around() {
        let pool = setup_pool(&three_backends());
        let strategy = RoundRobin::new();
        let healthy = pool.healthy_backends();

        assert_eq!(strategy.select(&healthy, None), Some(0));
        assert_eq!(strategy.select(&healthy, None), Some(1));
        assert_eq!(strategy.select(&healthy, None), Some(2));
        assert_eq!(strategy.select(&healthy, None), Some(0));
    }

    #[test]
    fn test_single_backend() {
        let pool = setup_pool(&[BackendConfig {
            addr: "0.0.0.0:8080".into(),
            weight: 1,
        }]);
        let strategy = RoundRobin::new();
        let healthy = pool.healthy_backends();

        assert_eq!(strategy.select(&healthy, None), Some(0));
        assert_eq!(strategy.select(&healthy, None), Some(0));
        assert_eq!(strategy.select(&healthy, None), Some(0));
    }

    #[test]
    fn test_empty_returns_none() {
        let strategy = RoundRobin::new();
        let empty: Vec<Arc<Backend>> = Vec::new();

        assert!(strategy.select(&empty, None).is_none());
    }
}
