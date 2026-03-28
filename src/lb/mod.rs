use std::net::SocketAddr;
use std::sync::Arc;

use crate::backend::Backend;

mod round_robin;

pub enum Algorithm {
    RoundRobin,
}

pub trait LoadBalancingStrategy: Send + Sync {
    fn select(&self, backends: &[Arc<Backend>], client_addr: Option<&SocketAddr>) -> Option<usize>;
}

pub fn create_strategy(algorithm: &Algorithm) -> Box<dyn LoadBalancingStrategy> {
    match algorithm {
        Algorithm::RoundRobin => Box::new(round_robin::RoundRobin::new()),
    }
}
