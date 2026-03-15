use serde::Deserialize;

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
    }
}
