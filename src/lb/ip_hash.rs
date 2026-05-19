use std::net::SocketAddr;
use std::sync::Arc;

use crate::backend::Backend;
use crate::lb::LoadBalancingStrategy;

#[derive(Debug)]
pub struct IpHash;

impl IpHash {
    pub const fn new() -> Self {
        Self
    }

    // Nginx' default hash function
    fn hash_ip(addr: &SocketAddr) -> usize {
        let mut hash: usize = 89;
        match addr.ip() {
            std::net::IpAddr::V4(ip) => {
                for &byte in &ip.octets() {
                    hash = (hash * 113 + byte as usize) % 6271;
                }
            }
            std::net::IpAddr::V6(ip) => {
                for &byte in &ip.octets() {
                    hash = (hash * 113 + byte as usize) % 6271;
                }
            }
        }
        hash
    }
}

impl Default for IpHash {
    fn default() -> Self {
        Self::new()
    }
}

impl LoadBalancingStrategy for IpHash {
    fn select(&self, backends: &[Arc<Backend>], client_addr: Option<&SocketAddr>) -> Option<usize> {
        if backends.is_empty() {
            return None;
        }

        let index = client_addr.map_or(0, |addr| Self::hash_ip(addr) % backends.len());

        Some(index)
    }

    fn algorithm(&self) -> &crate::config::Algorithm {
        &crate::config::Algorithm::IpHash
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
    fn test_deterministic_ipv4() {
        let pool = setup_pool(&three_backends());
        let strategy = IpHash::new();
        let addr: SocketAddr = "192.168.1.1:1234"
            .parse()
            .expect("Failed to parse SocketAddr");

        let first = strategy.select(&pool.healthy_backends(), Some(&addr));

        assert!(first.is_some());
        for _ in 0..100 {
            assert_eq!(
                strategy.select(&pool.healthy_backends(), Some(&addr)),
                first
            );
        }
    }

    #[test]
    fn test_deterministic_ipv6() {
        let pool = setup_pool(&three_backends());
        let strategy = IpHash::new();
        let addr: SocketAddr = "[2001:db8::1428:7ab]:1234"
            .parse()
            .expect("Failed to parse SocketAddr");

        let first = strategy.select(&pool.healthy_backends(), Some(&addr));

        assert!(first.is_some());
        for _ in 0..100 {
            assert_eq!(
                strategy.select(&pool.healthy_backends(), Some(&addr)),
                first
            );
        }
    }

    #[test]
    fn test_different_ips_distribute() {
        use std::collections::HashSet;

        let pool = setup_pool(&three_backends());
        let strategy = IpHash::new();
        let ips: Vec<SocketAddr> = (1..=100)
            .map(|i| {
                format!("11.0.0.{i}:5678")
                    .parse()
                    .expect("Failed to parse SocketAddr")
            })
            .collect();

        let unique_indices: HashSet<usize> = ips
            .iter()
            .map(|addr| {
                strategy
                    .select(&pool.healthy_backends(), Some(addr))
                    .expect("Failed to select backend")
            })
            .collect();

        assert!(
            unique_indices.len() > 1,
            "all IPs mapped to the same backend"
        );
    }

    #[test]
    fn test_none_addr_returns_zero() {
        let pool = setup_pool(&three_backends());
        let strategy = IpHash::new();

        assert_eq!(strategy.select(&pool.healthy_backends(), None), Some(0));
    }

    #[test]
    fn test_empty_returns_none() {
        let strategy = IpHash::new();
        let empty: Vec<Arc<Backend>> = Vec::new();

        assert!(strategy.select(&empty, None).is_none());
    }
}
