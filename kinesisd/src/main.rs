use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use band_core::responder::SimMode;
use kinesisd::actions::{Runner, ShellRunner};
use kinesisd::config::Config;
use kinesisd::daemon::Daemon;
use kinesisd::ipc::{self, Call, Handles};
use kinesisd::meta::{ClientFactory, FakeFactory, HttpFactory};
use kinesisd::store::Store;
use kinesisd::transport::{SimTransport, Transport};
use tokio::net::UnixListener;
use tokio::sync::mpsc;
use tracing_subscriber::EnvFilter;

const USAGE: &str = "usage: kinesisd [--dry-run] [--config PATH] [--scan]";

async fn shutdown_signal() {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("SIGTERM handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = terminate.recv() => {}
    }
}

async fn serve<T: Transport, R: Runner, F: ClientFactory>(
    daemon: Daemon<T, R, F>,
    listener: UnixListener,
) {
    let (calls, call_rx) = mpsc::channel::<Call>(32);
    let handles = Handles {
        calls,
        status: daemon.status(),
        semg: daemon.semg(),
    };
    let server = tokio::spawn(ipc::serve(listener, handles));
    daemon.run(call_rx, shutdown_signal()).await;
    server.abort();
}

async fn print_scan<T: Transport>(transport: &T) -> ExitCode {
    match transport.scan(Duration::from_secs(10)).await {
        Ok(bands) if bands.is_empty() => {
            println!(
                "no bands found (a band advertises while in pairing mode or when its owner is nearby)"
            );
            ExitCode::SUCCESS
        }
        Ok(bands) => {
            for band in bands {
                println!(
                    "{}  {}  rssi {}",
                    band.address,
                    band.name,
                    band.rssi.map_or("?".into(), |r| r.to_string())
                );
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("scan failed: {error}");
            ExitCode::FAILURE
        }
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_env("KINESIS_LOG").unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .init();
    let mut dry_run = false;
    let mut scan = false;
    let mut config_path = Config::default_path();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--dry-run" => dry_run = true,
            "--scan" => scan = true,
            "--config" => match args.next() {
                Some(path) => config_path = PathBuf::from(path),
                None => {
                    eprintln!("{USAGE}");
                    return ExitCode::from(2);
                }
            },
            "-h" | "--help" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("unknown argument {other}\n{USAGE}");
                return ExitCode::from(2);
            }
        }
    }

    if scan {
        return if dry_run {
            print_scan(&SimTransport::new(SimMode::Enrolled)).await
        } else {
            match kinesisd::bluez::BluezTransport::new().await {
                Ok(transport) => print_scan(&transport).await,
                Err(error) => {
                    eprintln!("Bluetooth unavailable: {error}");
                    ExitCode::FAILURE
                }
            }
        };
    }

    let socket = ipc::socket_path(dry_run);
    let listener = match ipc::bind(&socket) {
        Ok(listener) => listener,
        Err(error) => {
            tracing::error!("{}: {error}", socket.display());
            return ExitCode::FAILURE;
        }
    };
    let state_dir = if dry_run {
        Store::default_dir().with_file_name("kinesis-dry-run")
    } else {
        Store::default_dir()
    };
    let store = match Store::open(&state_dir) {
        Ok(store) => store,
        Err(error) => {
            tracing::error!("{}: {error}", state_dir.display());
            let _ = std::fs::remove_file(&socket);
            return ExitCode::FAILURE;
        }
    };
    tracing::info!("control socket {}", socket.display());
    if dry_run {
        let transport = SimTransport::new(SimMode::Enrolled);
        let factory = FakeFactory {
            device_point: Some(transport.band_point()),
            fail: None,
        };
        tracing::info!("dry run: simulated band, state in {}", state_dir.display());
        serve(
            Daemon::new(
                Arc::new(transport),
                ShellRunner::default(),
                factory,
                store,
                config_path,
            ),
            listener,
        )
        .await;
    } else {
        let transport = match kinesisd::bluez::BluezTransport::new().await {
            Ok(transport) => transport,
            Err(error) => {
                tracing::error!("Bluetooth unavailable: {error}");
                let _ = std::fs::remove_file(&socket);
                return ExitCode::FAILURE;
            }
        };
        serve(
            Daemon::new(
                Arc::new(transport),
                ShellRunner::default(),
                HttpFactory,
                store,
                config_path,
            ),
            listener,
        )
        .await;
    }
    let _ = std::fs::remove_file(&socket);
    ExitCode::SUCCESS
}
