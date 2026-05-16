use serde::Deserialize;
use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::Path;

// --- Error types ---
#[derive(thiserror::Error, Debug, PartialEq, Eq)]
pub enum ConfigValidationError {
    #[error("configuration must have at least one backend")]
    ZeroBackends,
    #[error("backend addresses must be unique")]
    DuplicateBackendAddrs,
    #[error("all backend addresses must be valid socket addresses")]
    InvalidAddrs,
}

#[derive(thiserror::Error, Debug)]
pub enum ConfigError {
    #[error("failed to read config file: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse config: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("config validation failed: {0}")]
    Validation(#[from] ConfigValidationError),
}

// --- Supporting types ---

#[derive(Deserialize, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum Algorithm {
    #[default]
    RoundRobin,
    WeightedRoundRobin,
    LeastConnections,
    IpHash,
    Random,
}

#[derive(Deserialize, PartialEq, Eq, Debug, Clone)]
#[serde(rename_all = "snake_case")]
pub struct BackendConfig {
    pub addr: String,
    #[serde(default = "default_weight")]
    pub weight: u32,
}

const fn default_weight() -> u32 {
    1
}

#[derive(Deserialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
#[serde(default)]
pub struct HealthCheckConfig {
    pub path: String,
    /// Interval between health checks in seconds
    pub interval: u64,
    /// Timeout for each health check request in seconds
    pub timeout: u64,
    pub healthy_threshold: u32,
    pub unhealthy_threshold: u32,
}

impl Default for HealthCheckConfig {
    fn default() -> Self {
        Self {
            path: String::from("/health"),
            interval: 5,
            timeout: 3,
            healthy_threshold: 3,
            unhealthy_threshold: 3,
        }
    }
}

#[derive(Deserialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
#[serde(default)]
/// Timeouts for backend connections in seconds
pub struct TimeoutConfig {
    pub connect: u64,
    pub read: u64,
    pub write: u64,
}

impl Default for TimeoutConfig {
    fn default() -> Self {
        Self {
            connect: 5,
            read: 30,
            write: 30,
        }
    }
}

// --- Primary type ---

#[derive(Deserialize, PartialEq, Eq, Debug)]
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
    pub fn validate(&self) -> Result<(), ConfigValidationError> {
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

    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let contents = std::fs::read_to_string(path)?;
        let config: Self = toml::from_str(&contents)?;
        config.validate()?;
        Ok(config)
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
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
            interval = 10
            timeout = 5
            healthy_threshold = 3
            unhealthy_threshold = 3

            [timeouts]
            connect = 5
            read = 30
            write = 30
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
                interval: 10,
                timeout: 5,
                healthy_threshold: 3,
                unhealthy_threshold: 3,
            },
            timeouts: TimeoutConfig {
                connect: 5,
                read: 30,
                write: 30,
            },
            max_connections: 1000,
        };

        assert_eq!(config.expect("failed to read config"), expected);
    }

    #[test]
    fn test_defaults_applied() {
        let config_str = r#"
            [[backends]]
            addr = "127.0.0.1:3000"
        "#;

        let config = toml::from_str::<Config>(config_str).expect("failed to read config");

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
    fn test_timeout_config_fields_accessible() {
        let config_str = r#"
            [[backends]]
            addr = "127.0.0.1:3000"

            [timeouts]
            connect = 10
            read = 60
            write = 45
        "#;

        let config = toml::from_str::<Config>(config_str).expect("failed to read config");

        assert_eq!(config.timeouts.connect, 10);
        assert_eq!(config.timeouts.read, 60);
        assert_eq!(config.timeouts.write, 45);
    }

    #[test]
    fn test_reject_zero_backends() {
        let config_str = "";

        let config = toml::from_str::<Config>(config_str).expect("failed to read config");

        let result = config.validate();

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

        let config = toml::from_str::<Config>(config_str).expect("failed to read config");

        let result = config.validate();

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

        let config = toml::from_str::<Config>(config_str).expect("failed to read config");

        let result = config.validate();

        assert_eq!(result.err(), Some(ConfigValidationError::InvalidAddrs));
    }

    #[test]
    fn test_reject_missing_fields() {
        let config_str = r"
            [[backends]]
            weight = 1
        ";

        let config = toml::from_str::<Config>(config_str);

        assert!(config.is_err());
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

            let config = toml::from_str::<Config>(&config_str).expect("failed to read config");

            assert_eq!(config.algorithm, expected_alg);
        }
    }

    #[test]
    fn test_read_config_from_file() {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let config_path = temp_dir.path().join("config.toml");

        let config_str = r#"
            [[backends]]
            addr = "0.0.0.0:3000"
        "#;

        let mut file = std::fs::File::create(&config_path).expect("failed to create config file");
        let _ = std::io::Write::write_all(&mut file, config_str.as_bytes());
        let config = Config::from_file(&config_path).expect("failed to read config from file");

        let expected = Config {
            backends: vec![BackendConfig {
                addr: String::from("0.0.0.0:3000"),
                weight: default_weight(),
            }],
            ..Config::default()
        };

        assert_eq!(config, expected);
    }

    #[test]
    fn test_from_file_returns_io_error_for_missing_file() {
        let result = Config::from_file("/nonexistent/path/config.toml");

        assert!(result.is_err());
        assert!(
            matches!(result.unwrap_err(), ConfigError::Io(_)),
            "Expected ConfigError::Io for missing file"
        );
    }

    #[test]
    fn test_from_file_returns_parse_error_for_invalid_toml() {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let config_path = temp_dir.path().join("bad.toml");

        std::fs::write(&config_path, "this is [[[not valid toml")
            .expect("failed to write file");

        let result = Config::from_file(&config_path);

        assert!(result.is_err());
        assert!(
            matches!(result.unwrap_err(), ConfigError::Parse(_)),
            "Expected ConfigError::Parse for invalid TOML"
        );
    }

    #[test]
    fn test_from_file_returns_validation_error_for_invalid_config() {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let config_path = temp_dir.path().join("empty.toml");

        // Valid TOML but no backends — triggers ZeroBackends validation
        std::fs::write(&config_path, "").expect("failed to write file");

        let result = Config::from_file(&config_path);

        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                ConfigError::Validation(ConfigValidationError::ZeroBackends)
            ),
            "Expected ConfigError::Validation(ZeroBackends)"
        );
    }
}
