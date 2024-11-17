use std::time::Duration;

use clap::Parser;
use tokio::signal::unix::{signal, SignalKind};
use tonic::transport::Server;
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let Args { address } = Args::parse();

    let (mut health_reporter, health_service) = tonic_health::server::health_reporter();
    let reflection_service = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(tonic_health::pb::FILE_DESCRIPTOR_SET)
        .build_v1()?;

    let mut shutdown_signal = signal(SignalKind::interrupt())?;

    info!("server listening on {}", address);
    Server::builder()
        .add_service(health_service)
        .add_service(reflection_service)
        .serve_with_shutdown(address.parse()?, async move {
            shutdown_signal.recv().await;
            info!("shutdown requested, marking as unhealthy");
            health_reporter
                .set_service_status("", tonic_health::ServingStatus::NotServing)
                .await;
            info!("waiting for grace period of 1s");
            tokio::time::sleep(Duration::from_secs(1)).await;
            info!("grace period complete, shutting down service");
        })
        .await?;

    info!("gracefully exiting");
    Ok(())
}

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "[::]:50051")]
    address: String,
}
