use crate::graphite::{Graphite, GraphiteConfig};
use log::{error, info, trace};
use std::fmt::Display;

pub struct Metrics {
    graphite: Graphite,
}

impl Metrics {
    pub fn graphite_based(graphite_config: GraphiteConfig) -> Result<Self, std::io::Error> {
        Graphite::new(graphite_config).map(|graphite| Metrics { graphite })
    }

    pub fn send_point_and_log_result<K: Display + Copy>(&self, point: K) -> () {
        match self.graphite.send_one_point(point) {
            Ok(_) => trace!("Successfully send {} to graphite", point),
            Err(err) => error!(
                "Error {} occurred during sending point {} to graphite",
                err, point
            ),
        }
    }
}
