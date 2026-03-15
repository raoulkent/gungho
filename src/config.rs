use serde::Deserialize;
use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::Path;

// --- Error types ---
#[derive(thiserror::Error, Debug, PartialEq)]
enum ConfigValidationError {
    #[error("configuration must have at least one backend")]
    ZeroBackends,
    #[error("backend addresses must be unique")]
    DuplicateBackendAddrs,
    #[error("all backend addresses must be valid socket addresses")]
    InvalidAddrs,
}

// --- Supporting types ---

#[derive(Deserialize, PartialEq, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum Algorithm {
    #[default]
    RoundRobin,
    WeightedRoundRobin,
    LeastConnections,
    IpHash,
    Random,
}

#[derive(Deserialize, PartialEq, Debug)]
#[serde(rename_all = "snake_case")]
pub struct BackendConfig {
    addr: String,
    #[serde(default = "default_weight")]
    weight: u32,
}

fn default_weight() -> u32 {
    1
}

#[derive(Deserialize, PartialEq, Debug)]
#[serde(rename_all = "snake_case")]
#[serde(default)]
pub struct HealthCheckConfig {
    path: String,
    interval_secs: u64,
    timeout_secs: u64,
    health_threshold: u32,
    unhealthy_threshold: u32,
}

impl Default for HealthCheckConfig {
    fn default() -> Self {
        HealthCheckConfig {
            path: String::from("/health"),
            interval_secs: 5,
            timeout_secs: 3,
            health_threshold: 3,
            unhealthy_threshold: 3,
        }
    }
}

#[derive(Deserialize, PartialEq, Debug)]
#[serde(rename_all = "snake_case")]
#[serde(default)]
pub struct TimeoutConfig {
    connect_timeout_secs: u64,
    read_timeout_secs: u64,
    write_timeout_secs: u64,
}

impl Default for TimeoutConfig {
    fn default() -> Self {
        TimeoutConfig {
            connect_timeout_secs: 5,
            read_timeout_secs: 30,
            write_timeout_secs: 30,
        }
    }
}

// --- Primary type ---

#[derive(Deserialize, PartialEq, Debug)]
#[serde(rename_all = "snake_case")]
#[serde(default)]
pub struct Config {
    pub listen_addr: String,
    pub admin_addr: String,
    pub backends: Vec<BackendConfig>,
    pub algorithm: Algorithm,
    pub health_check: HealthCheckConfig,
    pub timeouts: TimeoutConfig,
    pub max_connections: u32,
}

impl Config {
    fn validate(&self) -> Result<(), ConfigValidationError> {
        if self.backends.is_empty() {
            return Err(ConfigValidationError::ZeroBackends);
        }

        if self
            .backends
            .iter()
            .map(|b| &b.addr)
            .collect::<HashSet<_>>()
            .len()
            != self.backends.len()
        {
            return Err(ConfigValidationError::DuplicateBackendAddrs);
        }

        if self
            .backends
            .iter()
            .any(|b| b.addr.parse::<SocketAddr>().is_err())
        {
            return Err(ConfigValidationError::InvalidAddrs);
        }

        Ok(())
    }

    fn from_file(path: impl AsRef<Path>) -> Result<Self, Box<dyn std::error::Error>> {
        let contents = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&contents)?;
        config.validate()?;
        Ok(config)
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            listen_addr: String::from("0.0.0.0:8080"),
            admin_addr: String::from("0.0.0.0:9090"),
            backends: vec![],
            algorithm: Algorithm::default(),
            health_check: HealthCheckConfig::default(),
            timeouts: TimeoutConfig::default(),
            max_connections: 0, // 0 = unlimited
        }
    }
}

// --- Tests ---

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn test_parse_valid_config() {
        let config_str = r#"
            listen_addr = "0.0.0.0:8080"
            admin_addr = "0.0.0.0:9090"
            algorithm = "round_robin"
            max_connections = 1000

            [[backends]]
            addr = "127.0.0.1:3000"
            weight = 1

            [[backends]]
            addr = "127.0.0.1:3001"
            weight = 2

            [health_check]
            path = "/health"
            interval_secs = 10
            timeout_secs = 5
            health_threshold = 3
            unhealthy_threshold = 3

            [timeouts]
            connect_timeout_secs = 5
            read_timeout_secs = 30
            write_timeout_secs = 30
        "#;

        let config = toml::from_str::<Config>(config_str);

        let expected = Config {
            listen_addr: String::from("0.0.0.0:8080"),
            admin_addr: String::from("0.0.0.0:9090"),
            backends: vec![
                BackendConfig {
                    addr: String::from("127.0.0.1:3000"),
                    weight: 1,
                },
                BackendConfig {
                    addr: String::from("127.0.0.1:3001"),
                    weight: 2,
                },
            ],
            algorithm: Algorithm::RoundRobin,
            health_check: HealthCheckConfig {
                path: String::from("/health"),
                interval_secs: 10,
                timeout_secs: 5,
                health_threshold: 3,
                unhealthy_threshold: 3,
            },
            timeouts: TimeoutConfig {
                connect_timeout_secs: 5,
                read_timeout_secs: 30,
                write_timeout_secs: 30,
            },
            max_connections: 1000,
        };

        assert_eq!(config.unwrap(), expected);
    }

    #[test]
    fn test_defaults_applied() {
        let config_str = r#"
            [[backends]]
            addr = "127.0.0.1:3000"
        "#;

        let config = toml::from_str::<Config>(config_str).unwrap();

        let expected = Config {
            backends: vec![BackendConfig {
                addr: String::from("127.0.0.1:3000"),
                weight: default_weight(),
            }],
            ..Config::default()
        };

        assert_eq!(config, expected);
    }

    #[test]
    fn test_reject_zero_backends() {
        let config_str = "";

        let parsed = toml::from_str::<Config>(config_str).unwrap();

        let result = parsed.validate();

        assert_eq!(result.err(), Some(ConfigValidationError::ZeroBackends));
    }

    #[test]
    fn test_reject_duplicate_backend_addrs() {
        let config_str = r#"
            [[backends]]
            addr = "0.0.0.0:3000"

            [[backends]]
            addr = "0.0.0.0:3000"
        "#;

        let parsed = toml::from_str::<Config>(config_str).unwrap();

        let result = parsed.validate();

        assert_eq!(
            result.err(),
            Some(ConfigValidationError::DuplicateBackendAddrs)
        );
    }

    #[test]
    fn test_reject_invalid_backend_addrs() {
        let config_str = r#"
            [[backends]]
            addr = "invalid_addr"
        "#;

        let parsed = toml::from_str::<Config>(config_str).unwrap();

        let result = parsed.validate();

        assert_eq!(result.err(), Some(ConfigValidationError::InvalidAddrs));
    }

    #[test]
    fn test_reject_missing_fields() {
        let config_str = r#"
            [[backends]]
            weight = 1
        "#;

        let parsed = toml::from_str::<Config>(config_str);

        assert!(parsed.is_err());
    }

    #[test]
    fn test_all_algorithms_parsed() {
        let algorithms = vec![
            ("round_robin", Algorithm::RoundRobin),
            ("weighted_round_robin", Algorithm::WeightedRoundRobin),
            ("least_connections", Algorithm::LeastConnections),
            ("ip_hash", Algorithm::IpHash),
            ("random", Algorithm::Random),
        ];

        for (alg_str, expected_alg) in algorithms {
            let config_str = format!(
                r#"
                algorithm = "{alg_str}"

                [[backends]]
                addr = "0.0.0.0:3000"
            "#
            );

            let parsed = toml::from_str::<Config>(&config_str).unwrap();

            assert_eq!(parsed.algorithm, expected_alg);
        }
    }
}
