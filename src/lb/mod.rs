use std::net::SocketAddr;
use std::sync::Arc;

use crate::backend::Backend;
use crate::config::Algorithm;

mod round_robin;

pub trait LoadBalancingStrategy: Send + Sync {
    fn select(&self, backends: &[Arc<Backend>], client_addr: Option<&SocketAddr>) -> Option<usize>;
    fn algorithm(&self) -> &Algorithm;
}

pub fn create_strategy(algorithm: &Algorithm) -> Box<dyn LoadBalancingStrategy> {
    match algorithm {
        Algorithm::RoundRobin => Box::new(round_robin::RoundRobin::new()),
        _ => todo!(),
    }
}

#[cfg(test)]
mod tests {
    use crate::config::Algorithm;

    #[test]
    fn test_factory_creates_round_robin() {
        let strategy = super::create_strategy(&Algorithm::RoundRobin);
        assert_eq!(strategy.algorithm(), &Algorithm::RoundRobin);
    }
}
