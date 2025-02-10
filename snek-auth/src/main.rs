use crate::analytics::{Analytics, LaunchType};
use crate::metric_keys::SnekAuthMetricKey::{
    ADA2ADARequest as ADA2ADARequestKey, CaptchaVerificationFailed, CaptchaVerificationSuccess, CommonLaunch,
    CorrectFairLaunchOutput, EmptyAnalyticsInfoAboutPool, EmptyAuth, FailedFullVerification,
    FairLaunchWithNativeOutput, IncorrectFairLaunchOutput, IncorrectSignatureBasedRequestFormat,
    RequestReceived, SignatureVerificationFailed as SignatureVerificationFailedKey,
    SignatureVerificationSuccess, SuccessFullVerification,
};
use crate::re_captcha::{ReCaptcha, ReCaptchaSecret, ReCaptchaToken};
use crate::signature::Signature;
use crate::ProcessingErrorCode::{
    ADA2ADARequest, AuthEntityIsMissing, EmptyPoolInfo, IncorrectRequestStructure, PoolVerificationFailed,
    ReCapchaVerificationFailed, SignatureVerificationFailed, UnexpectedError,
};
use actix_cors::Cors;
use actix_web::error::InternalError;
use actix_web::http::header::ContentType;
use actix_web::web::Data;
use actix_web::{error, post, web, App, HttpResponse, HttpServer, Responder};
use bloom_offchain_cardano::orders::adhoc::beacon_from_oref;
use clap::Parser;
use cml_crypto::{Bip32PrivateKey, PrivateKey, RawBytesEncoding};
use derive_more::From;
use graphite::graphite::{Graphite, GraphiteConfig};
use graphite::metrics::Metrics;
use log::{error, info};
use spectrum_cardano_lib::{AssetClass, OutputRef};
use std::time::{SystemTime, UNIX_EPOCH};

mod analytics;
pub mod metric_keys;
pub mod re_captcha;
pub mod signature;

#[derive(serde::Serialize, From)]
struct SignatureHex(String);

#[derive(serde::Deserialize, Clone, Debug)]
struct AuthRequest {
    input_oref: OutputRef,
    order_index: u64,
    input_amount: u64,
    input_asset: AssetClass,
    output_asset: AssetClass,
    token: Option<ReCaptchaToken>,
    signature: Option<String>,
}

#[derive(serde::Serialize, Clone, Debug)]
enum ProcessingErrorCode {
    ReCapchaVerificationFailed,
    SignatureVerificationFailed,
    AuthEntityIsMissing,
    ADA2ADARequest,
    EmptyPoolInfo,
    IncorrectRequestStructure,
    PoolVerificationFailed,
    UnexpectedError,
}

impl Into<String> for ProcessingErrorCode {
    fn into(self) -> String {
        match self {
            ReCapchaVerificationFailed => "Captcha verification failed",
            SignatureVerificationFailed => "Signature verification failed",
            AuthEntityIsMissing => "Authentication failed",
            ADA2ADARequest => "ADA/ADA Request",
            EmptyPoolInfo => "Empty pool info",
            IncorrectRequestStructure => "Incorrect request",
            PoolVerificationFailed => "Verification failed",
            UnexpectedError => "Internal server error",
        }
        .to_string()
    }
}

#[derive(serde::Serialize, Clone, Debug)]
struct ProcessingError {
    message: String,
    code: ProcessingErrorCode,
}

#[derive(serde::Serialize)]
struct AuthResponse {
    signature: SignatureHex,
}

fn create_error_response(code: ProcessingErrorCode) -> HttpResponse {
    let parsing_error = ProcessingError {
        message: code.clone().into(),
        code,
    };
    let body = serde_json::to_string(&parsing_error).unwrap();
    HttpResponse::BadRequest()
        .content_type(ContentType::json())
        .body(body)
}

#[post("/auth")]
async fn auth(
    captcha: Data<ReCaptcha>,
    signature_verifier: Data<Signature>,
    analytics: Data<Analytics>,
    sk: Data<PrivateKey>,
    limits: Data<Limits>,
    metrics: Data<Metrics>,
    req: web::Json<AuthRequest>,
) -> impl Responder {
    let token_opt = req.output_asset.into_token().or(req.input_asset.into_token());
    metrics.send_point_and_log_result(RequestReceived);
    // Rules:
    // - if pool launch is `fair`:
    //  1) Captcha verification or Signature verification, if empty - error
    //  2) Token value verification:
    //      - If input is ADA:
    //          * If diff between pool launch and request is lt 3 min - 25 ADA
    //          * If diff between pool launch and request is lt 6 min and gte 3 min - 50 ADA
    //          * If diff between pool launch and request is lt 9 min and gte 6 min - 100 ADA
    //          * If diff between pool launch and request is gt 9 - no limit
    //      - If input is Token always true
    // - if pool launch is `common`:
    //  1) Captcha verification
    let system_time = SystemTime::now();
    let since_the_epoch = system_time.duration_since(UNIX_EPOCH).expect("Clocks not synced");
    info!(
        "Going to process request {:?} at {}",
        req,
        since_the_epoch.as_millis()
    );
    match (req.clone().token, req.clone().signature) {
        (Some(token), _) => {
            if !captcha.verify(token).await {
                metrics.send_point_and_log_result(CaptchaVerificationFailed);
                info!("Captcha verification failed");
                return create_error_response(ReCapchaVerificationFailed);
            } else {
                metrics.send_point_and_log_result(CaptchaVerificationSuccess)
            }
        }
        (_, Some(signature_from_request)) => {
            if let Some(verification_result_is_success) =
                signature_verifier.verify(req.clone().into(), signature_from_request)
            {
                if !verification_result_is_success {
                    info!("Signature verification failed for request {:?}", req.clone());
                    metrics.send_point_and_log_result(SignatureVerificationFailedKey);
                    return create_error_response(SignatureVerificationFailed);
                } else {
                    metrics.send_point_and_log_result(SignatureVerificationSuccess)
                }
            } else {
                metrics.send_point_and_log_result(IncorrectSignatureBasedRequestFormat);
                info!("Serialization failed for request: {:?}", req.clone());
                return create_error_response(IncorrectRequestStructure);
            }
        }
        _ => {
            metrics.send_point_and_log_result(EmptyAuth);
            return create_error_response(AuthEntityIsMissing);
        }
    };
    match token_opt {
        None => {
            metrics.send_point_and_log_result(ADA2ADARequestKey);
            return create_error_response(ADA2ADARequest);
        }
        Some(token) => match analytics.get_token_pool_info(token).await {
            Ok(pool_info) => {
                let pool_verification_result_is_success: bool = match pool_info.launch_type {
                    LaunchType::Fair => {
                        if req.output_asset.is_native() {
                            metrics.send_point_and_log_result(FairLaunchWithNativeOutput);
                            true
                        } else {
                            let pool_created_time = pool_info.created_on.as_secs();

                            let diff_between_order_and_pool_creation_in_mins =
                                (since_the_epoch.as_secs() as i64 - pool_info.created_on.as_secs() as i64)
                                    / 60;

                            info!(
                                "Difference between pool creation {} and request time is {} min.",
                                pool_created_time, diff_between_order_and_pool_creation_in_mins
                            );

                            let correct_output_value_for_fair_launch =
                                match diff_between_order_and_pool_creation_in_mins {
                                    less_than_3_min if less_than_3_min < 3 => {
                                        req.input_amount <= limits.three_min_limit
                                    }
                                    less_than_6_min if less_than_6_min < 6 => {
                                        req.input_amount <= limits.six_min_limit
                                    }
                                    less_than_9_min if less_than_9_min < 9 => {
                                        req.input_amount <= limits.nine_min_limit
                                    }
                                    more_than_9 if more_than_9 >= 9 => true,
                                    _ => false,
                                };

                            if correct_output_value_for_fair_launch {
                                metrics.send_point_and_log_result(CorrectFairLaunchOutput)
                            } else {
                                metrics.send_point_and_log_result(IncorrectFairLaunchOutput)
                            }

                            correct_output_value_for_fair_launch
                        }
                    }
                    LaunchType::Common => {
                        metrics.send_point_and_log_result(CommonLaunch);
                        true
                    }
                };
                if pool_verification_result_is_success {
                    let beacon = beacon_from_oref(
                        req.input_oref,
                        req.order_index,
                        req.input_amount,
                        req.input_asset,
                        req.output_asset,
                    );
                    let proof = sk.sign(beacon.to_raw_bytes());
                    let response = AuthResponse {
                        signature: proof.to_raw_hex().into(),
                    };
                    let body = serde_json::to_string(&response).unwrap();
                    metrics.send_point_and_log_result(SuccessFullVerification);
                    return HttpResponse::Ok().content_type(ContentType::json()).body(body);
                } else {
                    metrics.send_point_and_log_result(FailedFullVerification);
                    error!(
                        "pool_verification_result_is_success {} for request {:?}",
                        pool_verification_result_is_success, req
                    );
                    return create_error_response(PoolVerificationFailed);
                }
            }
            Err(err) => {
                metrics.send_point_and_log_result(EmptyAnalyticsInfoAboutPool);
                error!("Failed to fetch pool info: {}", err);
                return create_error_response(EmptyPoolInfo);
            }
        },
    }
}

#[derive(serde::Deserialize, Debug, Copy, Clone)]
#[serde(rename_all = "camelCase")]
struct Limits {
    three_min_limit: u64,
    six_min_limit: u64,
    nine_min_limit: u64,
}

#[derive(serde::Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct AppConfig {
    re_captcha_secret: ReCaptchaSecret,
    scoring_threshold: f64,
    secret_bech32: String,
    signature_secret: String,
    analytics_snek_url: String,
    limits: Limits,
    cache_size: usize,
    graphite: GraphiteConfig,
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let args = AppArgs::parse();

    log4rs::init_file(args.log4rs_path, Default::default()).unwrap();

    let raw_config = std::fs::File::open(args.config_path).expect("Cannot load configuration file");
    let config: AppConfig = serde_json::from_reader(raw_config).expect("Invalid configuration file");

    HttpServer::new(move || {
        let cors = Cors::default()
            .allow_any_origin()
            .allow_any_method()
            .allow_any_header();

        let json_config = web::JsonConfig::default().error_handler(|err, _req| {
            error!("Json parsing error: {}", err);
            InternalError::from_response(err, create_error_response(IncorrectRequestStructure)).into()
        });

        let re_captcha = Data::new(ReCaptcha::new(
            config.re_captcha_secret.clone(),
            config.scoring_threshold,
        ));
        let analytics = Data::new(Analytics::new(
            config.analytics_snek_url.clone(),
            config.cache_size,
        ));
        let signature_verifier = Data::new(Signature::new(config.signature_secret.clone()));
        let sk = Data::new(
            Bip32PrivateKey::from_bech32(config.secret_bech32.as_str())
                .expect("Invalid secret bech32")
                .to_raw_key(),
        );
        let metrics = Metrics::graphite_based(config.graphite.clone()).unwrap();
        App::new()
            .wrap(cors)
            .app_data(json_config)
            .app_data(re_captcha)
            .app_data(signature_verifier)
            .app_data(analytics)
            .app_data(sk)
            .app_data(Data::new(config.limits))
            .app_data(Data::new(metrics))
            .service(auth)
    })
    .bind((args.host, args.port))?
    .workers(8)
    .run()
    .await
}

#[derive(Parser)]
#[command(name = "snek-auth-server")]
#[command(author = "Spectrum Labs")]
#[command(version = "1.0.0")]
#[command(about = "Snek Auth Server", long_about = None)]
struct AppArgs {
    /// Path to the JSON configuration file.
    #[arg(long, short)]
    config_path: String,
    #[arg(long, short)]
    log4rs_path: String,
    #[arg(long)]
    host: String,
    #[arg(long)]
    port: u16,
}
