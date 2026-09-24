use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

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
    /// Claim a band in pairing mode for this computer (signs in to Meta if needed).
    Pair {
        /// A specific band's Bluetooth address (default: the first one found).
        #[arg(long)]
        address: Option<String>,
        /// Forget the currently paired band first.
        #[arg(long)]
        replace: bool,
    },
    /// Forget the band (keys, record, Bluetooth bond). Factory reset it afterwards.
    Forget,
    /// Sign in to Meta (needed once to claim a band). Opens Meta's own page.
    ///
    /// Always talks to the real Meta service; `--dry-run` only picks the
    /// dry-run state directory the session is saved in.
    Signin,
    /// Delete the saved Meta sign-in.
    Signout,
    /// Raw sEMG readings.
    Semg {
        #[command(subcommand)]
        what: SemgCommand,
    },
}

#[derive(Subcommand)]
enum SemgCommand {
    /// Live 8-channel view in the terminal.
    View,
    /// Record raw frames to a JSONL file until Ctrl-C (or --seconds).
    Record {
        file: PathBuf,
        #[arg(long)]
        seconds: Option<u64>,
    },
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
    let native = tokens.native_sso_token.clone();
    let accept = move |url: &str| metaauth::accepts_callback(url, &native);
    let Some(callback) = kinesisctl::signin::open(&metaauth::auth_entry_url(&tokens), accept)?
    else {
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

/// Whether a Meta sign-in is already saved (never touches whether it's still valid).
fn has_meta_session(store_dir: &std::path::Path) -> bool {
    kinesisd::store::Store::open(store_dir)
        .and_then(|store| store.load_meta_session())
        .is_ok_and(|session| session.is_some())
}

/// The band leaves pairing mode while the sign-in window is open (or once a
/// pairing attempt itself has failed): it has to be put back before pairing
/// can be started again.
fn prompt_to_re_enter_pairing_mode() {
    println!(
        "Put the band back in pairing mode (hold its button 3 s until it flashes), then press Enter."
    );
    let _ = std::io::stdin().read_line(&mut String::new());
}

fn pair(
    socket: &std::path::Path,
    address: Option<String>,
    replace: bool,
    dry_run: bool,
) -> ExitCode {
    let paired = runtime()
        .block_on(kinesisctl::client::status(socket))
        .map(|s| s.band.is_some());
    match paired {
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
        Ok(true) if !replace => {
            eprintln!(
                "a band is already paired. to claim a different (or reset) band: `kinesisctl pair --replace`"
            );
            return ExitCode::FAILURE;
        }
        Ok(true) => {
            // Sign in *before* forgetting the old band: if the sign-in window
            // is cancelled, the user still has a paired band afterwards.
            if !has_meta_session(&store_dir(dry_run))
                && let Err(error) = sign_in(&store_dir(dry_run))
            {
                eprintln!("{error}");
                return ExitCode::FAILURE;
            }
            if let Err(error) =
                runtime().block_on(kinesisctl::client::request(socket, &Request::Forget))
            {
                eprintln!("{error}");
                return ExitCode::FAILURE;
            }
            println!(
                "Forgot the old band. Now factory reset the band ({}),",
                band_core::ceremony::FACTORY_RESET_HINT
            );
            println!(
                "then hold its button for 3 seconds until it flashes (pairing mode), and press Enter."
            );
            let _ = std::io::stdin().read_line(&mut String::new());
        }
        Ok(false) => {
            println!(
                "Put the band in pairing mode: hold its button for 3 seconds until it flashes."
            );
        }
    }

    // At most one sign-in-and-retry: either the daemon refuses the initial
    // `Pair` outright (no session at all), or the pairing itself later fails
    // because the session it had turned out to be expired. Either way, the
    // band gave up on pairing mode while the sign-in window was open, so it
    // has to be put back before pairing is asked to start again.
    let mut signed_in_already = false;
    loop {
        if let Err(error) =
            runtime().block_on(kinesisctl::pair::start_pairing(socket, address.clone()))
        {
            if !signed_in_already
                && error.code.as_deref() == Some(kinesisd::ipc::CODE_NEEDS_SIGN_IN)
            {
                signed_in_already = true;
                if let Err(error) = sign_in(&store_dir(dry_run)) {
                    eprintln!("{error}");
                    return ExitCode::FAILURE;
                }
                prompt_to_re_enter_pairing_mode();
                continue;
            }
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
        match runtime().block_on(kinesisctl::pair::follow_pairing(socket, |stage| {
            println!("… {stage}");
        })) {
            Ok(kinesisctl::pair::Outcome::Paired { band }) => {
                println!("Paired with {band}. It's yours now — try a swipe.");
                return ExitCode::SUCCESS;
            }
            Ok(kinesisctl::pair::Outcome::PairedNotConnected { band }) => {
                println!(
                    "claimed {band}; it hasn't connected yet — kinesisd keeps retrying (kinesisctl status)"
                );
                return ExitCode::SUCCESS;
            }
            Ok(kinesisctl::pair::Outcome::NeedsSignIn) if !signed_in_already => {
                signed_in_already = true;
                if let Err(error) = sign_in(&store_dir(dry_run)) {
                    eprintln!("{error}");
                    return ExitCode::FAILURE;
                }
                prompt_to_re_enter_pairing_mode();
            }
            Ok(kinesisctl::pair::Outcome::NeedsSignIn) => {
                eprintln!(
                    "pairing failed: the meta session expired again; sign in and try `kinesisctl pair` again"
                );
                return ExitCode::FAILURE;
            }
            Ok(kinesisctl::pair::Outcome::Failed { message }) => {
                eprintln!("pairing failed: {message}");
                return ExitCode::FAILURE;
            }
            Err(error) => {
                eprintln!("{error}");
                return ExitCode::FAILURE;
            }
        }
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
        Command::Pair { address, replace } => pair(&socket, address, replace, cli.dry_run),
        Command::Forget => simple(&socket, Request::Forget),
        Command::Signin => report(sign_in(&store_dir(cli.dry_run))),
        Command::Signout => report(sign_out(&store_dir(cli.dry_run))),
        Command::Semg {
            what: SemgCommand::View,
        } => report(runtime().block_on(kinesisctl::semg::view(&socket))),
        Command::Semg {
            what: SemgCommand::Record { file, seconds },
        } => {
            match runtime().block_on(kinesisctl::semg::record(
                &socket,
                &file,
                seconds.map(Duration::from_secs),
            )) {
                Ok(frames) => {
                    println!("recorded {frames} frames to {}", file.display());
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("{error}");
                    ExitCode::FAILURE
                }
            }
        }
    }
}
