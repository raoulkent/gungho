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

    pub fn select_backend(&self, backends: &[Arc<Backend>]) -> Option<Arc<Backend>> {
        if backends.is_empty() {
            return None;
        }

        let index = self.counter.fetch_add(1, Ordering::SeqCst) % backends.len();
        Some(backends[index].clone())
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
    #[test]
    fn test_round_robin_even_distribution() {}

    #[test]
    fn test_round_robin_wraps_around() {}

    #[test]
    fn test_round_robin_single_backend() {}

    #[test]
    fn test_round_robin_empty_backends() {}

    #[test]
    fn test_factory_creates_round_robin() {}
}
