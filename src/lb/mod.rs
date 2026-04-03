use std::net::SocketAddr;
use std::sync::Arc;

use crate::backend::Backend;
use crate::config::Algorithm;

mod least_connections;
mod round_robin;
mod weighted_round_robin;

pub trait LoadBalancingStrategy: Send + Sync {
    fn select(&self, backends: &[Arc<Backend>], client_addr: Option<&SocketAddr>) -> Option<usize>;
    fn algorithm(&self) -> &Algorithm;
}

pub fn create_strategy(algorithm: &Algorithm) -> Box<dyn LoadBalancingStrategy> {
    match algorithm {
        Algorithm::RoundRobin => Box::new(round_robin::RoundRobin::new()),
        Algorithm::WeightedRoundRobin => Box::new(weighted_round_robin::WeightedRoundRobin::new()),
        Algorithm::LeastConnections => Box::new(least_connections::LeastConnections::new()),
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

    #[test]
    fn test_factory_creates_weighted_round_robin() {
        let strategy = super::create_strategy(&Algorithm::WeightedRoundRobin);
        assert_eq!(strategy.algorithm(), &Algorithm::WeightedRoundRobin);
    }

    #[test]
    fn test_factory_creates_least_connections() {
        let strategy = super::create_strategy(&Algorithm::LeastConnections);
        assert_eq!(strategy.algorithm(), &Algorithm::LeastConnections);
    }
}
