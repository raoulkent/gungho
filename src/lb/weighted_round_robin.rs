use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use crate::backend::Backend;
use crate::lb::LoadBalancingStrategy;

#[derive(Debug)]
pub struct WeightedRoundRobin {
    credits: Mutex<HashMap<SocketAddr, i64>>,
}

impl WeightedRoundRobin {
    pub fn new() -> Self {
        Self {
            credits: Mutex::new(HashMap::new()),
        }
    }

    pub fn adjust_credits(&self, backend: &Backend, delta: i64) {
        *self
            .credits
            .lock()
            .expect("Failed to acquire lock on credits HashMap")
            .entry(backend.get_addr())
            .or_insert(0) += delta;
    }
}

/// Selects a backend using the Smooth Weighted Round Robin (SWRR) algorithm.
///
/// On each call:
/// 1. Each backend's weight is added to its running credit
/// 2. The backend with the highest credit is selected
/// 3. The total weight of all backends is subtracted from the winner's credit
///
/// This produces a smooth, proportional distribution identical to nginx's SWRR.
impl LoadBalancingStrategy for WeightedRoundRobin {
    fn select(
        &self,
        backends: &[Arc<Backend>],
        _client_addr: Option<&SocketAddr>,
    ) -> Option<usize> {
        if backends.is_empty() {
            return None;
        }

        let mut credits = self
            .credits
            .lock()
            .expect("Failed to acquire lock on credits HashMap");

        let mut total_weight: i64 = 0;
        let mut best_index = 0;
        let mut best_credit = i64::MIN;

        for (i, backend) in backends.iter().enumerate() {
            let w = i64::from(backend.get_weight());
            total_weight += w;
            let credit = credits.entry(backend.get_addr()).or_insert(0);
            *credit += w;
            if *credit > best_credit {
                best_credit = *credit;
                best_index = i;
            }
        }
        // Subtract total_weight from the winner
        if let Some(winner_credit) = credits.get_mut(&backends[best_index].get_addr()) {
            *winner_credit -= total_weight;
        }

        drop(credits); // Explicitly drop the lock before returning
        Some(best_index)
    }

    fn algorithm(&self) -> &crate::config::Algorithm {
        &crate::config::Algorithm::WeightedRoundRobin
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
    fn test_weighted_distribution() {
        let pool = setup_pool(&[
            BackendConfig {
                addr: "0.0.0.0:8080".into(),
                weight: 5,
            },
            BackendConfig {
                addr: "0.0.0.1:8080".into(),
                weight: 1,
            },
            BackendConfig {
                addr: "0.0.0.2:8080".into(),
                weight: 1,
            },
        ]);
        let strategy = WeightedRoundRobin::new();

        let mut counts = [0u32; 3];
        for _ in 0..700 {
            let idx = strategy
                .select(&pool.healthy_backends(), None)
                .expect("No backend selected");
            counts[idx] += 1;
        }

        assert_eq!(counts[0], 500);
        assert_eq!(counts[1], 100);
        assert_eq!(counts[2], 100);
    }

    #[test]
    fn test_equal_weights() {
        let pool = setup_pool(&three_backends());
        let strategy = WeightedRoundRobin::new();

        let mut counts = [0u32; 3];
        for _ in 0..300 {
            let idx = strategy
                .select(&pool.healthy_backends(), None)
                .expect("No backend selected");
            counts[idx] += 1;
        }

        assert_eq!(counts[0], 100);
        assert_eq!(counts[1], 100);
        assert_eq!(counts[2], 100);
    }

    #[test]
    fn test_single_backend() {
        let pool = setup_pool(&[BackendConfig {
            addr: "0.0.0.0:8080".into(),
            weight: 1,
        }]);
        let strategy = WeightedRoundRobin::new();

        let mut counts = [0u32; 1];
        for _ in 0..100 {
            let idx = strategy
                .select(&pool.healthy_backends(), None)
                .expect("No backend selected");
            counts[idx] += 1;
        }

        assert_eq!(counts[0], 100);
    }

    #[test]
    fn test_empty_returns_none() {
        let strategy = WeightedRoundRobin::new();
        let empty: Vec<Arc<Backend>> = Vec::new();

        assert!(strategy.select(&empty, None).is_none());
    }
}
