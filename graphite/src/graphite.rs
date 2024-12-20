use std::fmt::Display;
use std::net::UdpSocket;

#[derive(serde::Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GraphiteConfig {
    host: String,
    port: u64,
    prefix: String,
}

pub struct Graphite {
    prefix: String,
    graphite_host: String,
    graphite_port: u64,
    socket: UdpSocket,
}

impl Graphite {
    pub fn new(graphite_config: GraphiteConfig) -> Result<Self, std::io::Error> {
        let socket = UdpSocket::bind("0.0.0.0:0")?;
        Ok(Graphite {
            prefix: graphite_config.prefix,
            graphite_host: graphite_config.host,
            graphite_port: graphite_config.port,
            socket,
        })
    }

    pub fn send_one_point<K: Display>(&self, metric: K) -> Result<(), std::io::Error> {
        let wrapped_metric = format!("{}.{}:1|c\n", self.prefix, metric);
        let server_addr = format!("{}:{}", self.graphite_host, self.graphite_port);
        self.socket.send_to(wrapped_metric.as_bytes(), server_addr)?;
        Ok(())
    }
}
