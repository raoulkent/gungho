use std::net::SocketAddr;
use std::sync::Arc;

use crate::backend::Backend;
use crate::lb::LoadBalancingStrategy;

#[derive(Debug)]
pub struct LeastConnections;

impl LeastConnections {
    pub const fn new() -> Self {
        Self
    }
}

impl LoadBalancingStrategy for LeastConnections {
    fn select(
        &self,
        backends: &[Arc<Backend>],
        _client_addr: Option<&SocketAddr>,
    ) -> Option<usize> {
        if backends.is_empty() {
            return None;
        }
        let mut least_index = 0;
        let mut least_conns = usize::MAX;

        for (i, backend) in backends.iter().enumerate() {
            let conns = backend.get_active_connections();
            if conns < least_conns {
                least_conns = conns;
                least_index = i;
            }
        }
        Some(least_index)
    }

    fn algorithm(&self) -> &crate::config::Algorithm {
        &crate::config::Algorithm::LeastConnections
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::BackendPool;
    use crate::config::BackendConfig;
    use crate::lb::LoadBalancingStrategy;

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

    // - test_selects_lowest_connections — [5, 2, 8] → index 1
    // - test_tie_picks_first — [3, 3, 3] → index 0
    // - test_all_zero — [0, 0, 0] → index 0
    // - test_empty_returns_none — [] → None

    #[test]
    fn test_least_connections() {
        let backends = three_backends();
        let pool = setup_pool(&backends);
        let strategy = super::LeastConnections::new();

        for _ in 0..10 {
            let index = strategy
                .select(&pool.healthy_backends(), None)
                .expect("Failed to select backend");
            pool.healthy_backends()[index].increment_connections();
        }

        assert_eq!(pool.healthy_backends()[0].get_active_connections(), 4);
        assert_eq!(pool.healthy_backends()[1].get_active_connections(), 3);
        assert_eq!(pool.healthy_backends()[2].get_active_connections(), 3);
    }

    #[test]
    fn test_selects_lowest_connections() {
        let pool = setup_pool(&three_backends());
        let strategy = LeastConnections::new();
        let backends = pool.all_backends();

        for _ in 0..5 {
            backends[0].increment_connections();
        } // 5 conns
        for _ in 0..2 {
            backends[1].increment_connections();
        } // 2 conns
        for _ in 0..8 {
            backends[2].increment_connections();
        } // 8 conns

        let selected = strategy.select(backends, None);

        // Picks index 1 (lowest at 2)
        assert_eq!(selected, Some(1));
    }

    #[test]
    fn test_tie_picks_first() {}

    #[test]
    fn test_all_zero() {}

    #[test]
    fn test_empty_returns_none() {
        let strategy = super::LeastConnections::new();
        let empty: Vec<Arc<Backend>> = Vec::new();
        assert_eq!(strategy.select(&empty, None), None);
    }
}
