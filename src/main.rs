use std::{net::SocketAddr, time::Duration};

use clap::Parser;
use tonic::transport::Server;
use tracing::{info, warn};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let Args {
        address,
        grace_period_ms,
    } = Args::parse();
    let address: SocketAddr = address.parse()?;

    let (mut health_reporter, health_service) = tonic_health::server::health_reporter();
    let reflection_service = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(tonic_health::pb::FILE_DESCRIPTOR_SET)
        .build_v1()?;

    let (tx, mut shutdown) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        info!("recv SIGINT, latching shutdown signal");
        tx.send_replace(true);
    });

    // This future will resolve when the server shuts down organically (either via a graceful serve_with_shutdown
    // or by encountering an error).
    let organic = tokio::spawn({
        let mut shutdown = shutdown.clone();
        info!("server listening on {}", address);
        Server::builder()
            .add_service(health_service)
            .add_service(reflection_service)
            .serve_with_shutdown(address, async move {
                let _ = shutdown.wait_for(|&is_shutdown| is_shutdown).await;
                info!("marking as unhealthy to discourage clients");
                health_reporter
                    .set_service_status("", tonic_health::ServingStatus::NotServing)
                    .await;
                info!("no longer accepting new connections");
            })
    });

    // This future will resolve after the process receives a SIGINT and the grace period has expired.
    // When it resolves, we need to shut down ungracefully.
    let ungraceful = async move {
        let _ = shutdown.wait_for(|&is_shutdown| is_shutdown).await;
        if let Some(grace_period_ms) = grace_period_ms {
            info!("waiting up to {grace_period_ms}ms for clients to disconnect",);
            tokio::time::sleep(Duration::from_millis(grace_period_ms)).await;
        } else {
            info!("waiting forever for clients to disconnect");
            let () = std::future::pending().await;
        }
    };

    tokio::select! {
        r = organic => {
            r??; // if we hit any kind of organic error with the server, bubble that up
            info!("all clients gracefully disconnected, exiting");
        },
        _ = ungraceful => warn!("grace period exhausted, forcefully shutting down connections"),
    };
    Ok(())
}

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "[::]:50051")]
    address: String,

    #[arg(long)]
    grace_period_ms: Option<u64>,
}
