use crate::graphite::Graphite;
use bloom_offchain::execution_engine::liquidity_book::core::Make;
use log::{error, info};
use std::io::Error;
use strum_macros::Display;

pub struct Metrics {
    graphite: Graphite,
}

#[derive(Display, Copy, Clone)]
pub enum MetricKey {
    IncorrectRequestReceived,
    RequestReceived,
    EmptyAuth,
    CaptchaVerificationFailed,
    CaptchaVerificationSuccess,
    SignatureVerificationFailed,
    SignatureVerificationSuccess,
    IncorrectSignatureBasedRequestFormat,
    ADA2ADARequest,
    EmptyAnalyticsInfoAboutPool,
    FairLaunchWithNativeOutput,
    CorrectFairLaunchOutput,
    IncorrectFairLaunchOutput,
    CommonLaunch,
    SuccessFullVerification,
    FailedFullVerification,
}

impl Metrics {
    pub fn new(graphite: Graphite) -> Metrics {
        Metrics { graphite }
    }

    pub fn send_point_and_log_result(&self, point: MetricKey) -> () {
        match self.graphite.send_one_point(point) {
            Ok(_) => info!("Successfully send {} to graphite", point),
            Err(err) => error!(
                "Error {} occurred during sending point {} to graphite",
                err, point
            ),
        }
    }
}
