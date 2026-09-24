use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use kinesisd::config::DialTarget;
use kinesisd::ipc::{HandChoice, Request};
use kinesisd::status::LinkState;

#[derive(Parser)]
#[command(
    name = "kinesisctl",
    version,
    about = "Control the Meta Neural Band daemon (kinesisd)"
)]
struct Cli {
    /// Talk to the `kinesisd --dry-run` daemon instead of the real one.
    #[arg(long, global = true)]
    dry_run: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, ValueEnum)]
enum DialArg {
    Volume,
    Brightness,
    Off,
    Cycle,
}

#[derive(Clone, Copy, ValueEnum)]
enum HandArg {
    Left,
    Right,
}

#[derive(Subcommand)]
enum Command {
    /// Show the band's state.
    Status,
    /// Connect to the paired band.
    Connect,
    /// Disconnect from the band.
    Disconnect,
    /// Connect if idle, else disconnect.
    ToggleConnection,
    /// Stop running gesture actions (the band stays connected).
    Pause,
    /// Run gesture actions again.
    Resume,
    /// Pause or resume.
    Toggle,
    /// Choose what the pinch-and-turn dial controls.
    Dial { target: DialArg },
    /// Tell the band which wrist it's on.
    Hand { hand: HandArg },
    /// Forget the band (keys, record, Bluetooth bond). Factory reset it afterwards.
    Forget,
}

fn socket(dry_run: bool) -> PathBuf {
    kinesisd::ipc::socket_path(dry_run)
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
}

fn print_status_of(response: kinesisd::ipc::Response) {
    if let Some(message) = &response.message {
        println!("{message}");
    }
    if let Some(status) = &response.status {
        println!("{}", kinesisctl::text::describe(status));
    }
}

fn simple(socket: &std::path::Path, request: Request) -> ExitCode {
    match runtime().block_on(kinesisctl::client::request(socket, &request)) {
        Ok(response) => {
            print_status_of(response);
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let socket = socket(cli.dry_run);
    match cli.command {
        Command::Status => simple(&socket, Request::Status),
        Command::Connect => simple(&socket, Request::Connect),
        Command::Disconnect => simple(&socket, Request::Disconnect),
        Command::ToggleConnection => {
            let state = runtime()
                .block_on(kinesisctl::client::status(&socket))
                .map(|s| s.state);
            match state {
                Ok(LinkState::Idle) => simple(&socket, Request::Connect),
                Ok(_) => simple(&socket, Request::Disconnect),
                Err(error) => {
                    eprintln!("{error}");
                    ExitCode::FAILURE
                }
            }
        }
        Command::Pause => simple(&socket, Request::Pause),
        Command::Resume => simple(&socket, Request::Resume),
        Command::Toggle => simple(&socket, Request::TogglePause),
        Command::Dial { target } => simple(
            &socket,
            match target {
                DialArg::Volume => Request::Dial {
                    target: DialTarget::Volume,
                },
                DialArg::Brightness => Request::Dial {
                    target: DialTarget::Brightness,
                },
                DialArg::Off => Request::Dial {
                    target: DialTarget::Off,
                },
                DialArg::Cycle => Request::CycleDial,
            },
        ),
        Command::Hand { hand } => simple(
            &socket,
            Request::Hand {
                hand: match hand {
                    HandArg::Left => HandChoice::Left,
                    HandArg::Right => HandChoice::Right,
                },
            },
        ),
        Command::Forget => simple(&socket, Request::Forget),
    }
}
