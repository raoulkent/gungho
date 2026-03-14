use serde::Deserialize;

#[derive(Deserialize, PartialEq, Debug)]
#[serde(rename_all = "snake_case")]
pub struct Config {
    listen_addr: String,
    admin_addr: String,
    backends: Vec<BackendConfig>,
    algorithm: Algorithm,
    health_check: HealthCheckConfig,
    timeouts: TimeoutConfig,
    max_connections: u32,
}

#[derive(Deserialize, PartialEq, Debug)]
#[serde(rename_all = "snake_case")]
pub struct BackendConfig {
    addr: String,
    weight: u32,
}

#[derive(Deserialize, PartialEq, Debug)]
#[serde(rename_all = "snake_case")]
pub struct HealthCheckConfig {
    path: String,
    interval_secs: u64,
    timeout_secs: u64,
    health_threshold: u32,
    unhealthy_threshold: u32,
}

#[derive(Deserialize, PartialEq, Debug)]
#[serde(rename_all = "snake_case")]
pub struct TimeoutConfig {
    connect_timeout_secs: u64,
    read_timeout_secs: u64,
    write_timeout_secs: u64,
}

#[derive(Deserialize, PartialEq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Algorithm {
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
        listen_addr: "0.0.0.0:8080"
        admin_addr: "0.0.0.0:9090"
        backends:
        - addr: "127.0.0.1:3000"
          weight: 1
        - addr: "127.0.0.1:3001"
          weight: 2
        algorithm: round_robin
        health_check:
            path: "/health"
            interval_secs: 10
            timeout_secs: 5
            health_threshold: 3
            unhealthy_threshold: 3
        timeouts:
            connect_timeout_secs: 5
            read_timeout_secs: 30
            write_timeout_secs: 30
        max_connections: 1000
        "#;

        let read_config = serde_yaml_ng::from_str::<Config>(config_str);

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

        assert_eq!(read_config.unwrap(), expected);
    }
}
