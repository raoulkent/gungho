use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::backend::Backend;
use crate::lb::LoadBalancingStrategy;

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
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::backend::{Backend, BackendPool};
    use crate::config::BackendConfig;
    use crate::lb::LoadBalancingStrategy;

    #[test]
    fn test_round_robin_even_distribution() {
        let backends = [
            BackendConfig {
                addr: "0.0.0.0:8080".into(),
                weight: 1,
            },
            BackendConfig {
                addr: "0.0.0.1:8080".into(),
                weight: 2,
            },
            BackendConfig {
                addr: "0.0.0.2:8080".into(),
                weight: 3,
            },
        ];

        let pool = BackendPool::from_config(&backends).expect("BackendPool ");

        let strategy = super::RoundRobin::new();

        for i in 0..300 {
            let selected = strategy
                .select(&pool.healthy_backends(), None)
                .expect("Should select a backend");
            let expected_index = i % pool.all_backends().len();
            assert_eq!(selected, expected_index,);
        }
    }

    #[test]
    fn test_round_robin_wraps_around() {
        let backends = [
            BackendConfig {
                addr: "0.0.0.0:8080".into(),
                weight: 1,
            },
            BackendConfig {
                addr: "0.0.0.1:8080".into(),
                weight: 2,
            },
            BackendConfig {
                addr: "0.0.0.2:8080".into(),
                weight: 3,
            },
        ];

        let pool = BackendPool::from_config(&backends).expect("BackendPool");
        let healthy = pool.healthy_backends();
        let strategy = super::RoundRobin::new();

        assert_eq!(strategy.select(&healthy, None), Some(0));
        assert_eq!(strategy.select(&healthy, None), Some(1));
        assert_eq!(strategy.select(&healthy, None), Some(2));
        assert_eq!(strategy.select(&healthy, None), Some(0));
    }

    #[test]
    fn test_round_robin_single_backend() {
        let backends = [BackendConfig {
            addr: "0.0.0.0:8080".into(),
            weight: 1,
        }];

        let pool = BackendPool::from_config(&backends).expect("BackendPool");
        let healthy = pool.healthy_backends();
        let strategy = super::RoundRobin::new();

        assert_eq!(strategy.select(&healthy, None), Some(0));
        assert_eq!(strategy.select(&healthy, None), Some(0));
        assert_eq!(strategy.select(&healthy, None), Some(0));
    }

    #[test]
    fn test_round_robin_empty_backends() {
        let empty: Vec<Arc<Backend>> = Vec::new();
        let strategy = super::RoundRobin::new();

        assert_eq!(strategy.select(&empty, None), None);
    }
}
