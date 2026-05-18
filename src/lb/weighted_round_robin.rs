use std::net::SocketAddr;
use std::sync::atomic::AtomicI64;
use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock};

use crate::backend::Backend;
use crate::lb::LoadBalancingStrategy;

pub struct WeightedRoundRobin {
    credits: OnceLock<Vec<AtomicI64>>,
}

impl std::fmt::Debug for WeightedRoundRobin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WeightedRoundRobin").finish()
    }
}

impl WeightedRoundRobin {
    pub const fn new() -> Self {
        Self {
            credits: OnceLock::new(),
        }
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

        let credits = self
            .credits
            .get_or_init(|| (0..backends.len()).map(|_| AtomicI64::new(0)).collect());

        let mut total_weight: i64 = 0;
        let mut best_index = 0;
        let mut best_credit = i64::MIN;

        for (i, backend) in backends.iter().enumerate() {
            let w = i64::from(backend.get_weight());
            total_weight += w;
            let credit = &credits[i];
            credit.fetch_add(w, Ordering::Relaxed);
            if credit.load(Ordering::Relaxed) > best_credit {
                best_credit = credit.load(Ordering::Relaxed);
                best_index = i;
            }
        }
        // Subtract total_weight from the winner
        if let Some(winner_credit) = credits.get(best_index) {
            winner_credit.fetch_sub(total_weight, Ordering::Relaxed);
        }

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
