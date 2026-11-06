use std::sync::Arc;

use alloy::{
    network::{Ethereum, EthereumWallet},
    primitives::Address,
    providers::ProviderBuilder,
    signers::local::PrivateKeySigner,
};
use base64::{Engine, prelude::BASE64_STANDARD_NO_PAD};
use diesel::{PgConnection, r2d2::ConnectionManager};
use ed25519_dalek::SigningKey;
use end2::{
    AppState, CometBftDeviceKeyService, DeviceKeyService, DirectoryKey, EthDeviceKeyService,
    ForgingDeviceKeyService, MaliciousDeviceKeyService,
};
use mimalloc::MiMalloc;
use opentelemetry::trace::TracerProvider;
use opentelemetry_sdk::metrics::PeriodicReader;
use r2d2::Pool;
use tokio::net::TcpListener;
use tracing_subscriber::prelude::*;

use end2::{
    App, AuthService, DbAuthService, DbDeviceKeyService, DbMessageRelayService, DbOtkService,
    MessageRelayService, OAuthHandler, OAuthInfo, OtkService,
};

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

const DISCORD_AUTHORIZE_URL: &str = "https://discord.com/oauth2/authorize";
const DISCORD_TOKEN_URL: &str = "https://discord.com/api/oauth2/token";

// see https://developers.google.com/identity/openid-connect/openid-connect
const GOOGLE_AUTHORIZE_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

fn init_telemetry() {
    let span_exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .build()
        .expect("span exporter");
    let tracer_provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
        .with_batch_exporter(span_exporter)
        .build();

    let log_exporter = opentelemetry_otlp::LogExporter::builder()
        .with_tonic()
        .build()
        .expect("log exporter");
    let logger_provider = opentelemetry_sdk::logs::SdkLoggerProvider::builder()
        .with_batch_exporter(log_exporter)
        .build();

    let tracer = tracer_provider.tracer("end2");
    let otel_trace_layer = tracing_opentelemetry::layer().with_tracer(tracer);
    let otel_logs_layer =
        opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge::new(&logger_provider);

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().json())
        .with(otel_trace_layer)
        .with(otel_logs_layer)
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    opentelemetry::global::set_tracer_provider(tracer_provider.clone());

    let metric_exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_tonic()
        .build()
        .expect("metric exporter");
    let reader = PeriodicReader::builder(metric_exporter).build();
    let meter_provider = opentelemetry_sdk::metrics::SdkMeterProvider::builder()
        .with_reader(reader)
        .build();
    opentelemetry::global::set_meter_provider(meter_provider.clone());
}

async fn setup_eth_device_keys(
    pool: Pool<ConnectionManager<PgConnection>>,
) -> Arc<dyn DeviceKeyService> {
    let rpc_url = std::env::var("ETH_RPC_URL").expect("ETH_RPC_URL must be set");
    let relayer_key = std::env::var("ETH_RELAYER_KEY").expect("ETH_RELAYER_KEY must be set");
    let contract_address = std::env::var("CONTRACT_ADDRESS")
        .expect("CONTRACT_ADDRESS must be set")
        .parse::<Address>()
        .expect("invalid contract address");

    let signer: PrivateKeySigner = relayer_key.parse().expect("invalid relayer private key");
    let wallet = EthereumWallet::from(signer);
    let provider = ProviderBuilder::new_with_network::<Ethereum>()
        .wallet(wallet)
        .connect(&rpc_url)
        .await
        .expect("ethereum provider");

    Arc::new(EthDeviceKeyService::new(
        Arc::new(provider),
        contract_address,
        pool,
    ))
}

fn setup_comet_device_keys(
    pool: Pool<ConnectionManager<PgConnection>>,
    signing_key: Arc<SigningKey>,
    directory: Arc<DirectoryKey>,
) -> Arc<dyn DeviceKeyService> {
    let rpc_url = std::env::var("COMET_RPC_URL").expect("COMET_RPC_URL must be set");
    Arc::new(CometBftDeviceKeyService::new(
        rpc_url,
        signing_key,
        directory,
        pool,
    ))
}

fn setup_db_device_keys(pool: Pool<ConnectionManager<PgConnection>>) -> Arc<dyn DeviceKeyService> {
    Arc::new(DbDeviceKeyService::new(pool))
}

fn setup_malicious_device_keys(
    pool: Pool<ConnectionManager<PgConnection>>,
    signing_key: Arc<SigningKey>,
    directory: Arc<DirectoryKey>,
) -> Arc<dyn DeviceKeyService> {
    let rpc_url = std::env::var("COMET_RPC_URL").expect("COMET_RPC_URL must be set");
    Arc::new(MaliciousDeviceKeyService::new(
        rpc_url,
        signing_key,
        directory,
        pool,
    ))
}

fn setup_forging_device_keys(
    pool: Pool<ConnectionManager<PgConnection>>,
    signing_key: Arc<SigningKey>,
    directory: Arc<DirectoryKey>,
) -> Arc<dyn DeviceKeyService> {
    let rpc_url = std::env::var("COMET_RPC_URL").expect("COMET_RPC_URL must be set");
    Arc::new(ForgingDeviceKeyService::new(
        rpc_url,
        signing_key,
        directory,
        pool,
    ))
}

#[tokio::main]
async fn main() -> Result<(), std::io::Error> {
    dotenvy::dotenv().ok();
    init_telemetry();

    // IP and port to run server on
    let ip = std::env::var("BACKEND_IP").expect("BACKEND_IP must be set");
    let port: u16 = std::env::var("BACKEND_PORT")
        .expect("BACKEND_PORT must be set")
        .parse()
        .expect("BACKEND_PORT must be in range 0-65535");

    // Load signing key that the backend uses to sign keys distributed by the blockchain
    // so that in a way it acts as a CA
    let signing_key = std::env::var("SERVER_SIGNING_KEY").expect("SERVER_SIGNING_KEY must be set");
    let signing_key_bytes = BASE64_STANDARD_NO_PAD
        .decode(signing_key)
        .expect("invalid base64");
    let signing_key = SigningKey::from_bytes(
        signing_key_bytes
            .as_slice()
            .try_into()
            .expect("invalid key length"),
    );

    // Load variables for Discord OAuth2 flow
    let client_id =
        std::env::var("DISCORD_OAUTH_CLIENT_ID").expect("DISCORD_OAUTH_CLIENT_ID must be set");
    let client_secret =
        std::env::var("DISCORD_OAUTH_SECRET").expect("DISCORD_OAUTH_SECRET must be set");
    let redirect =
        std::env::var("DISCORD_OAUTH_REDIRECT").expect("DISCORD_OAUTH_REDIRECT must be set");
    let discord_oauth = OAuthHandler::new(OAuthInfo {
        client_id,
        client_secret,
        redirect,
        auth_url: DISCORD_AUTHORIZE_URL.to_string(),
        token_url: DISCORD_TOKEN_URL.to_string(),
        scopes: vec!["identify"],
    });

    // Load variables for Google OAuth2 flow
    let client_id =
        std::env::var("GOOGLE_OAUTH_CLIENT_ID").expect("GOOGLE_OAUTH_CLIENT_ID must be set");
    let client_secret =
        std::env::var("GOOGLE_OAUTH_SECRET").expect("GOOGLE_OAUTH_SECRET must be set");
    let redirect =
        std::env::var("GOOGLE_OAUTH_REDIRECT").expect("GOOGLE_OAUTH_REDIRECT must be set");
    let google_oauth = OAuthHandler::new(OAuthInfo {
        client_id,
        client_secret,
        redirect,
        auth_url: GOOGLE_AUTHORIZE_URL.to_string(),
        token_url: GOOGLE_TOKEN_URL.to_string(),
        scopes: vec!["openid", "email", "profile"],
    });

    // Load PostgreSQL database
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let manager = ConnectionManager::<PgConnection>::new(&database_url);
    let pool = r2d2::Pool::builder()
        .build(manager)
        .expect("Failed to connect to Postgres");

    let mut oauth = std::collections::HashMap::new();
    oauth.insert("discord", discord_oauth);
    oauth.insert("google", google_oauth);

    let signing_key = Arc::new(signing_key);

    let directory = Arc::new(
        DirectoryKey::from_base64(
            &std::env::var("VRF_SECRET_KEY").expect("VRF_SECRET_KEY must be set"),
        )
        .expect("invalid VRF_SECRET_KEY"),
    );
    tracing::info!(vrf_public_key = %directory.public_key_base64(), "loaded VRF directory key");

    // let device_keys = setup_comet_device_keys(pool.clone(), signing_key.clone(), directory.clone());
    // let device_keys = setup_db_device_keys(pool.clone());
    // let device_keys = setup_eth_device_keys(pool.clone()).await;
    // let device_keys = setup_malicious_device_keys(pool.clone(), signing_key.clone(), directory.clone());
    let device_keys =
        setup_forging_device_keys(pool.clone(), signing_key.clone(), directory.clone());

    let auth = Arc::new(DbAuthService::new(pool.clone(), oauth));
    let otks = Arc::new(DbOtkService::new(pool.clone()));
    let relay = Arc::new(DbMessageRelayService::new(pool.clone()));

    let app_state = AppState::new(auth, device_keys, otks, relay, pool, signing_key, directory);

    let app = App::new(app_state);
    let listener = TcpListener::bind((ip, port)).await.expect("TcpListener");
    app.run(listener).await
}
