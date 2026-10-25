use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use air_gesturesd::config::DialTarget;
use air_gesturesd::ipc::{HandChoice, Request};
use air_gesturesd::status::LinkState;
use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(
    name = "air-gestures",
    version,
    about = "Control the Meta Neural Band daemon (air-gesturesd)"
)]
struct Cli {
    /// Talk to the `air-gesturesd --dry-run` daemon instead of the real one.
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
    /// Switch profiles: next, previous, or a profile's name.
    Profile { to: String },
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
    /// Save the owner key and band record to a file, to use the band from another
    /// device (`air-gestures import`, or Import in the Android app).
    Export {
        /// Where to write it.
        #[arg(default_value = air_gestures::transfer::DEFAULT_FILE)]
        path: PathBuf,
        /// Overwrite the file if it exists.
        #[arg(long)]
        force: bool,
    },
    /// Use the band's owner key from another device (a file from `air-gestures
    /// export` or the Android app's Export, or a raw owner.key).
    Import {
        file: PathBuf,
        /// Don't ask before replacing the key on this computer.
        #[arg(long)]
        yes: bool,
    },
    /// Calibrate palm positions (down, up) for the band and the hand it's on.
    Calibrate {
        /// Forget the calibration (back to the measured right-hand one; none for the left hand).
        #[arg(long)]
        reset: bool,
    },
    /// Teach the turn and the six drags (one pinch each) for the band and the hand it's on.
    TeachDrags {
        /// Forget them (back to the measured right-hand ones; turns only for the left hand).
        #[arg(long)]
        reset: bool,
    },
    /// Drawings: teach one (three tries), list them, delete one.
    Draw {
        #[command(subcommand)]
        what: DrawCommand,
    },
    /// Raw sEMG readings.
    Semg {
        #[command(subcommand)]
        what: SemgCommand,
    },
}

#[derive(Subcommand)]
enum DrawCommand {
    /// Teach a drawing by drawing it three times.
    Teach { name: String },
    /// The taught drawings.
    List,
    /// Forget a drawing.
    Delete { name: String },
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
    air_gesturesd::ipc::socket_path(dry_run)
}

/// Where air-gesturesd keeps its state; the dry-run daemon uses its own directory
/// (as in air-gesturesd's main), so a dry run never touches the real Meta session.
fn store_dir(dry_run: bool) -> PathBuf {
    let dir = air_gesturesd::store::Store::default_dir();
    if dry_run {
        dir.with_file_name("air-gestures-dry-run")
    } else {
        dir
    }
}

/// The whole sign-in: tokens query → window → callback check → decrypt → login → save.
/// Never prints or logs the tokens.
fn sign_in(store_dir: &std::path::Path) -> Result<(), String> {
    use air_gestures::metaauth;
    println!(
        "Opening Meta's sign-in page. You type your password there; air-gestures only keeps the resulting session token."
    );
    let tokens = runtime().block_on(metaauth::tokens_query())?;
    let native = tokens.native_sso_token.clone();
    let accept = move |url: &str| metaauth::accepts_callback(url, &native);
    let Some(callback) = air_gestures::signin::open(&metaauth::auth_entry_url(&tokens), accept)?
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
    air_gesturesd::store::Store::open(store_dir)
        .and_then(|store| store.save_meta_session(&session))
        .map_err(|error| format!("could not save the sign-in: {error}"))?;
    println!("Signed in to Meta.");
    Ok(())
}

fn sign_out(store_dir: &std::path::Path) -> Result<(), String> {
    air_gesturesd::store::Store::open(store_dir)
        .and_then(|store| store.delete_meta_session())
        .map_err(|error| format!("could not delete the sign-in: {error}"))?;
    println!("Signed out of Meta.");
    Ok(())
}

fn export(store_dir: &std::path::Path, path: &std::path::Path, force: bool) -> Result<(), String> {
    let store = air_gesturesd::store::Store::open(store_dir)
        .map_err(|error| format!("could not open {}: {error}", store_dir.display()))?;
    air_gestures::transfer::export(&store, path, force)?;
    println!("Exported the band to {}", path.display());
    println!("Keep it private: anyone holding this file can control the band.");
    Ok(())
}

fn import(
    socket: &std::path::Path,
    store_dir: &std::path::Path,
    file: &std::path::Path,
    yes: bool,
) -> Result<(), String> {
    let bytes = std::fs::read(file)
        .map_err(|error| format!("could not read {}: {error}", file.display()))?;
    let imported = air_gestures::transfer::parse(&bytes)?;
    let store = air_gesturesd::store::Store::open(store_dir)
        .map_err(|error| format!("could not open {}: {error}", store_dir.display()))?;
    // An unreadable key counts as one: it's still what gets replaced.
    let has_key = !matches!(store.load_identity(), Ok(None)) || store.has_pending();
    if has_key && !yes {
        print!(
            "This replaces the owner key on this computer. If only this computer holds it, the band has to be claimed again to get it back. Continue? [y/N] "
        );
        use std::io::Write;
        let _ = std::io::stdout().flush();
        let mut answer = String::new();
        let _ = std::io::stdin().read_line(&mut answer);
        if !matches!(answer.trim(), "y" | "Y" | "yes") {
            return Err("import cancelled".into());
        }
    }
    air_gestures::transfer::import(&store, &imported)?;
    match &imported.band {
        Some(band) => println!("Imported the owner key for {}.", band.name),
        None if store.load_band().is_ok_and(|band| band.is_some()) => {
            println!(
                "Imported the owner key (the file had no band record; the current one is kept)."
            )
        }
        None => {
            println!(
                "Imported the owner key. The file had no band record, so air-gesturesd doesn't know which band to connect to: import a file from `air-gestures export` that has one."
            );
            return Ok(());
        }
    }
    // The daemon reads the key on each connect: reconnect so it uses the new one.
    if runtime()
        .block_on(air_gestures::client::status(socket))
        .is_err()
    {
        println!("air-gesturesd isn't running; it uses the new key when it starts.");
        return Ok(());
    }
    runtime()
        .block_on(async {
            air_gestures::client::request(socket, &Request::Disconnect).await?;
            air_gestures::client::request(socket, &Request::Connect).await
        })
        .map(print_status_of)
        .map_err(|error| format!("imported, but reconnecting failed: {error}"))
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

fn print_status_of(response: air_gesturesd::ipc::Response) {
    if let Some(message) = &response.message {
        println!("{message}");
    }
    if let Some(status) = &response.status {
        println!("{}", air_gestures::text::describe(status));
    }
}

/// Whether a Meta sign-in is already saved (never touches whether it's still valid).
fn has_meta_session(store_dir: &std::path::Path) -> bool {
    air_gesturesd::store::Store::open(store_dir)
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
        .block_on(air_gestures::client::status(socket))
        .map(|s| s.band.is_some());
    match paired {
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
        Ok(true) if !replace => {
            eprintln!(
                "a band is already paired. to claim a different (or reset) band: `air-gestures pair --replace`"
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
                runtime().block_on(air_gestures::client::request(socket, &Request::Forget))
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
            runtime().block_on(air_gestures::pair::start_pairing(socket, address.clone()))
        {
            if !signed_in_already
                && error.code.as_deref() == Some(air_gesturesd::ipc::CODE_NEEDS_SIGN_IN)
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
        match runtime().block_on(air_gestures::pair::follow_pairing(socket, |stage| {
            println!("… {stage}");
        })) {
            Ok(air_gestures::pair::Outcome::Paired { band }) => {
                println!("Paired with {band}. It's yours now — try a swipe.");
                return ExitCode::SUCCESS;
            }
            Ok(air_gestures::pair::Outcome::PairedNotConnected { band }) => {
                println!(
                    "claimed {band}; it hasn't connected yet — air-gesturesd keeps retrying (air-gestures status)"
                );
                return ExitCode::SUCCESS;
            }
            Ok(air_gestures::pair::Outcome::NeedsSignIn) if !signed_in_already => {
                signed_in_already = true;
                if let Err(error) = sign_in(&store_dir(dry_run)) {
                    eprintln!("{error}");
                    return ExitCode::FAILURE;
                }
                prompt_to_re_enter_pairing_mode();
            }
            Ok(air_gestures::pair::Outcome::NeedsSignIn) => {
                eprintln!(
                    "pairing failed: the meta session expired again; sign in and try `air-gestures pair` again"
                );
                return ExitCode::FAILURE;
            }
            Ok(air_gestures::pair::Outcome::Failed { message }) => {
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
    match runtime().block_on(air_gestures::client::request(socket, &request)) {
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
            let status = runtime()
                .block_on(air_gestures::client::status(&socket))
                .ok();
            // Ignore write failures (e.g. a broken pipe): waybar may already
            // have stopped reading, and that's not a reason to panic.
            let _ = writeln!(
                std::io::stdout(),
                "{}",
                air_gestures::waybar::line(status.as_ref())
            );
            ExitCode::SUCCESS
        }
        Command::Status {
            waybar: true,
            follow: true,
        } => runtime().block_on(async {
            use std::io::Write;
            let mut lines = air_gestures::waybar::next_lines(socket.clone());
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
                .block_on(air_gestures::client::status(&socket))
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
        Command::Profile { to } => simple(&socket, Request::Profile { to }),
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
        Command::Export { path, force } => report(export(&store_dir(cli.dry_run), &path, force)),
        Command::Import { file, yes } => {
            report(import(&socket, &store_dir(cli.dry_run), &file, yes))
        }
        Command::Calibrate { reset: true } => simple(&socket, Request::SetPalm { palm: None }),
        Command::Calibrate { reset: false } => report(
            runtime().block_on(air_gestures::teach::calibrate(&socket, &mut air_gestures::teach::Terminal)).map(|_| ()),
        ),
        Command::TeachDrags { reset: true } => simple(&socket, Request::SetDrags { drags: None }),
        Command::TeachDrags { reset: false } => {
            report(runtime().block_on(air_gestures::teach::teach_drags(&socket, &mut air_gestures::teach::Terminal)))
        }
        Command::Draw { what } => {
            let dir = air_gesturesd::drawings::default_dir(cli.dry_run);
            match what {
                DrawCommand::Teach { name } => report(runtime().block_on(air_gestures::teach::teach_drawing(
                    &socket,
                    &dir,
                    &name,
                    &mut air_gestures::teach::Terminal,
                ))),
                DrawCommand::List => report(air_gestures::teach::list_drawings(&dir).map(|list| println!("{list}"))),
                DrawCommand::Delete { name } => report(air_gestures::teach::delete_drawing(&dir, &name).map(|()| println!("deleted \"{name}\""))),
            }
        }
        Command::Semg {
            what: SemgCommand::View,
        } => report(runtime().block_on(air_gestures::semg::view(&socket))),
        Command::Semg {
            what: SemgCommand::Record { file, seconds },
        } => {
            match runtime().block_on(air_gestures::semg::record(
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Command {
        Cli::try_parse_from(std::iter::once("air-gestures").chain(args.iter().copied())).unwrap().command
    }

    #[test]
    fn the_teaching_commands_parse() {
        assert!(matches!(parse(&["calibrate"]), Command::Calibrate { reset: false }));
        assert!(matches!(parse(&["calibrate", "--reset"]), Command::Calibrate { reset: true }));
        assert!(matches!(parse(&["--dry-run", "calibrate"]), Command::Calibrate { .. }));
        assert!(matches!(parse(&["teach-drags"]), Command::TeachDrags { reset: false }));
        assert!(matches!(parse(&["teach-drags", "--reset"]), Command::TeachDrags { reset: true }));
        assert!(matches!(parse(&["draw", "teach", "lightbulb"]), Command::Draw { what: DrawCommand::Teach { name } } if name == "lightbulb"));
        assert!(matches!(parse(&["draw", "list"]), Command::Draw { what: DrawCommand::List }));
        assert!(matches!(parse(&["draw", "delete", "z"]), Command::Draw { what: DrawCommand::Delete { name } } if name == "z"));
        assert!(Cli::try_parse_from(["air-gestures", "draw", "teach"]).is_err(), "a name is needed");
    }
}
