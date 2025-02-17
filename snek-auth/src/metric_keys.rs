use strum_macros::Display;

#[derive(Display, Copy, Clone)]
pub enum SnekAuthMetricKey {
    IncorrectRequestReceived,
    RequestReceived,
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
