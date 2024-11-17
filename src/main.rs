use std::time::Duration;

use clap::Parser;
use tokio::signal::unix::{signal, SignalKind};
use tonic::transport::Server;
use tracing::{info, warn};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let Args { address } = Args::parse();

    let (mut health_reporter, health_service) = tonic_health::server::health_reporter();
    let reflection_service = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(tonic_health::pb::FILE_DESCRIPTOR_SET)
        .build_v1()?;

    let mut shutdown_signal = {
        let (tx, rx) = tokio::sync::watch::channel(0);
        let mut sig = signal(SignalKind::interrupt())?;
        tokio::spawn(async move {
            let _ = sig.recv().await;
            tx.send_replace(1);
            info!("waiting for grace period of 5s");
            tokio::time::sleep(Duration::from_millis(5_000)).await;
            info!("grace period complete, shutting down service");
            tx.send_replace(2);
        });
        rx
    };

    info!("server listening on {}", address);
    let mut srv_sig = shutdown_signal.clone();
    let graceful_exit = Server::builder()
        .add_service(health_service)
        .add_service(reflection_service)
        .serve_with_shutdown(address.parse()?, async move {
            let _ = srv_sig.wait_for(|&v| v == 1).await;
            info!("shutdown requested, marking as unhealthy");
            health_reporter
                .set_service_status("", tonic_health::ServingStatus::NotServing)
                .await;
        });

    tokio::select! {
        _ = graceful_exit => {
            info!("gracefully exiting");
        }
        _ = shutdown_signal.wait_for(|&v| v == 2) => {
            warn!("grace period exhausted, forcefully shutting down")
        }
    };
    Ok(())
}

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "[::]:50051")]
    address: String,
}
