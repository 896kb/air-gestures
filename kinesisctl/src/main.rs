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
    Status {
        /// Print a waybar JSON line instead of text.
        #[arg(long)]
        waybar: bool,
        /// Keep printing a line on every change (waybar `exec` without `interval`).
        #[arg(long, requires = "waybar")]
        follow: bool,
    },
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
    /// Sign in to Meta (needed once to claim a band). Opens Meta's own page.
    Signin,
    /// Delete the saved Meta sign-in.
    Signout,
}

fn socket(dry_run: bool) -> PathBuf {
    kinesisd::ipc::socket_path(dry_run)
}

/// Where kinesisd keeps its state; the dry-run daemon uses its own directory
/// (as in kinesisd's main), so a dry run never touches the real Meta session.
fn store_dir(dry_run: bool) -> PathBuf {
    let dir = kinesisd::store::Store::default_dir();
    if dry_run {
        dir.with_file_name("kinesis-dry-run")
    } else {
        dir
    }
}

/// The whole sign-in: tokens query → window → callback check → decrypt → login → save.
/// Never prints or logs the tokens.
fn sign_in(store_dir: &std::path::Path) -> Result<(), String> {
    use kinesisctl::metaauth;
    println!(
        "Opening Meta's sign-in page. You type your password there; kinesis only keeps the resulting session token."
    );
    let tokens = runtime().block_on(metaauth::tokens_query())?;
    let Some(callback) = kinesisctl::signin::open(&metaauth::auth_entry_url(&tokens))? else {
        return Err("sign-in cancelled".into());
    };
    let (token, blob) = metaauth::parse_callback(&callback).unwrap_or((None, None));
    let Some(blob) = blob.filter(|b| {
        !b.is_empty() && metaauth::callback_matches(token.as_deref(), &tokens.native_sso_token)
    }) else {
        return Err("meta didn't confirm this sign-in. try again.".into());
    };
    let session = runtime().block_on(async {
        let frl = metaauth::decrypt_blob(&blob, &tokens.native_sso_token).await?;
        metaauth::login(&frl).await
    })?;
    kinesisd::store::Store::open(store_dir)
        .and_then(|store| store.save_meta_session(&session))
        .map_err(|error| format!("could not save the sign-in: {error}"))?;
    println!("Signed in to Meta.");
    Ok(())
}

fn sign_out(store_dir: &std::path::Path) -> Result<(), String> {
    kinesisd::store::Store::open(store_dir)
        .and_then(|store| store.delete_meta_session())
        .map_err(|error| format!("could not delete the sign-in: {error}"))?;
    println!("Signed out of Meta.");
    Ok(())
}

fn report(result: Result<(), String>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
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
        Command::Status { waybar: false, .. } => simple(&socket, Request::Status),
        Command::Status {
            waybar: true,
            follow: false,
        } => {
            use std::io::Write;
            let status = runtime().block_on(kinesisctl::client::status(&socket)).ok();
            // Ignore write failures (e.g. a broken pipe): waybar may already
            // have stopped reading, and that's not a reason to panic.
            let _ = writeln!(
                std::io::stdout(),
                "{}",
                kinesisctl::waybar::line(status.as_ref())
            );
            ExitCode::SUCCESS
        }
        Command::Status {
            waybar: true,
            follow: true,
        } => runtime().block_on(async {
            use std::io::Write;
            let mut lines = kinesisctl::waybar::next_lines(socket.clone());
            while let Some(line) = lines.recv().await {
                let mut out = std::io::stdout().lock();
                if writeln!(out, "{line}").and_then(|_| out.flush()).is_err() {
                    break; // waybar went away
                }
            }
            ExitCode::SUCCESS
        }),
        Command::Connect => simple(&socket, Request::Connect),
        Command::Disconnect => simple(&socket, Request::Disconnect),
        Command::ToggleConnection => {
            let state = runtime()
                .block_on(kinesisctl::client::status(&socket))
                .map(|s| s.state);
            match state {
                Ok(LinkState::Idle) => simple(&socket, Request::Connect),
                Ok(LinkState::Pairing) => {
                    println!("a pairing is running");
                    ExitCode::SUCCESS
                }
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
        Command::Signin => report(sign_in(&store_dir(cli.dry_run))),
        Command::Signout => report(sign_out(&store_dir(cli.dry_run))),
    }
}
