use std::{sync::Arc, time::Duration};

use clap::Parser;
use tokio::{
    signal::unix::{signal, SignalKind},
    sync::Semaphore,
};
use tonic::transport::Server;
use tracing::{info, warn};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let Args {
        address,
        grace_period_ms,
    } = Args::parse();

    let (mut health_reporter, health_service) = tonic_health::server::health_reporter();
    let reflection_service = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(tonic_health::pb::FILE_DESCRIPTOR_SET)
        .build_v1()?;

    let shutdown = shutdown_signal()?;

    let graceful_exit = tokio::spawn({
        let shutdown = shutdown.clone();
        info!("server listening on {}", address);
        let srv = Server::builder()
            .add_service(health_service)
            .add_service(reflection_service);
        srv.serve_with_shutdown(address.parse()?, async move {
            shutdown.wait().await;
            info!("no longer accepting new connections");
        })
    });

    shutdown.wait().await;
    info!("marking as unhealthy to discourage clients");
    health_reporter
        .set_service_status("", tonic_health::ServingStatus::NotServing)
        .await;

    let ungraceful_exit = async move {
        if let Some(grace_period_ms) = grace_period_ms {
            info!("waiting up to {grace_period_ms}ms for clients to disconnect",);
            tokio::time::sleep(Duration::from_millis(grace_period_ms)).await;
        } else {
            info!("waiting forever for clients to disconnect");
            let () = std::future::pending().await;
        }
    };

    tokio::select! {
        _ = graceful_exit => {
            info!("all clients gracefully disconnected, exiting");
        }
        _ = ungraceful_exit => {
            warn!("grace period exhausted, forcefully shutting down connections")
        }
    };
    Ok(())
}

// A signal that will be notified when the server should start shutting down
// For now, that can be triggered by sending a SIGINT.
fn shutdown_signal() -> anyhow::Result<Latch> {
    let shutdown = Latch::new();
    let tx = shutdown.clone();
    let mut sig = signal(SignalKind::interrupt())?;
    tokio::spawn(async move {
        let _ = sig.recv().await;
        info!("recv SIGINT, latching shutdown signal");
        tx.latch();
    });
    Ok(shutdown)
}

// I'm sure there's some better async latch primitive around, but I haven't found one.
// Things I have tried:
//   - using tokio::sync::Notify: had to be super careful about missing notifications before calling `.notified()`
//   - using tokio::sync::oneshot: can't clone the receiver, so it only works for a single waiter
//   - using tokio::sync::watch: works fine, just awkward to have to store an initial value and use `wait_for`
// The semaphore approach is a bit awkward as well. We initialize it with no permits, so `acquire()` will never
// succeed. We close it when we want to wake waiters, and they'll wake by getting an error.
#[derive(Clone)]
struct Latch(Arc<Semaphore>);
impl Latch {
    fn new() -> Self {
        Self(Arc::new(Semaphore::new(0)))
    }
    fn latch(&self) {
        self.0.close();
    }
    async fn wait(&self) {
        let _ = self.0.acquire().await;
    }
}

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "[::]:50051")]
    address: String,

    #[arg(long)]
    grace_period_ms: Option<u64>,
}
