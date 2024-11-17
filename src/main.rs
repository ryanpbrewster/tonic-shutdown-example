use std::{sync::Arc, time::Duration};

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

    // A signal that will be notified when the server should start shutting down
    // For now, that can be triggered by sending a SIGINT.
    let shutdown = {
        let shutdown = Arc::new(tokio::sync::Notify::new());
        let tx = shutdown.clone();
        let mut sig = signal(SignalKind::interrupt())?;
        tokio::spawn(async move {
            let _ = sig.recv().await;
            tx.notify_waiters();
        });
        shutdown
    };

    info!("server listening on {}", address);
    let srv = Server::builder()
        .add_service(health_service)
        .add_service(reflection_service);

    let graceful_exit = {
        let shutdown = shutdown.clone();
        srv.serve_with_shutdown(address.parse()?, async move {
            shutdown.notified().await;
            health_reporter
                .set_service_status("", tonic_health::ServingStatus::NotServing)
                .await;
        })
    };

    let ungraceful_exit = async move {
        shutdown.notified().await;
        info!("waiting up to 5s for clients to disconnect");
        tokio::time::sleep(Duration::from_millis(5_000)).await;
    };

    tokio::select! {
        _ = graceful_exit => {
            info!("gracefully exiting");
        }
        _ = ungraceful_exit => {
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
