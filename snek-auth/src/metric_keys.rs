use bloom_offchain::execution_engine::liquidity_book::core::Make;
use graphite::graphite::Graphite;
use log::{error, info};
use std::io::Error;
use strum_macros::Display;

#[derive(Display, Copy, Clone)]
pub enum SnekAuthMetricKey {
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
