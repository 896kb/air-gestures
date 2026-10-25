//! The supervisor: keeps one link to the paired band (reconnecting with
//! backoff), routes its events through band-core's controls to the runner, reloads
//! the config and the taught drawings, applies what's taught for the band and hand in use,
//! runs pairings and teaching steps, and answers control-socket calls.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use band_core::ceremony::FACTORY_RESET_HINT;
use band_core::controls::draw::Library;
use band_core::controls::pinch::Prototypes;
use band_core::controls::{Builtin, Controls, Output, PalmCalibration, PalmSet, Profiles};
use band_core::events::{Event, Hand};
use band_core::session::BandSession;
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio::task::JoinHandle;

use crate::actions::Runner;
use crate::clock::Clock;
use crate::config::{Config, DialTarget, HandSetting, Level};
use crate::drawings;
use crate::ipc::{Call, HandChoice, Request, Response, Taught};
use crate::link::{LinkCommand, LinkEnd, LinkEvent, LinkOutcome, run_link};
use crate::meta::ClientFactory;
use crate::pairing::{PairFailure, pair_band};
use crate::status::{LinkState, PairResult, Status};
use crate::store::{BandRecord, HandCalibration, IdentitySource, Store, StoredDrags};
use crate::teach::{Done, Wait};
use crate::transport::Transport;

const TICK: Duration = Duration::from_millis(250);
/// Raw sEMG is turned off after this long without a viewer attached.
const SEMG_IDLE: f64 = 3.0;
/// Identity result code: the band is enrolled to a different key.
const DIFFERENT_KEY: u32 = 0x1043;
const ALREADY_PAIRED: &str = "a band is already paired — forget it first (`air-gestures forget`, then factory reset the band)";
const PAIRING_RUNNING: &str = "a pairing is running; wait for it to finish";
/// What a request that needs the band says without one.
pub const CONNECT_FIRST: &str = "connect the band first (`air-gestures connect`)";

struct ActiveLink {
    commands: mpsc::Sender<LinkCommand>,
    events: mpsc::Receiver<LinkEvent>,
    task: JoinHandle<LinkOutcome>,
    /// The link authenticates with the pending key from an unconfirmed claim.
    pending: bool,
}

type PairingTask = JoinHandle<Result<BandRecord, PairFailure>>;

/// A teaching step waiting for the band, and where its answer goes.
struct Waiting {
    wait: Wait,
    reply: oneshot::Sender<Response>,
}

/// Why a waiting step ends by itself.
enum WaitEnd {
    TimedOut,
    /// The client hung up (its reply can't be delivered).
    Gone,
}

/// Resolves when the waiting step's deadline passes or its client goes; never without one.
async fn wait_ends(waiting: &mut Option<Waiting>, clock: Clock) -> WaitEnd {
    match waiting {
        Some(waiting) => {
            let deadline = waiting.wait.deadline();
            tokio::select! {
                () = until(Some(deadline), clock) => WaitEnd::TimedOut,
                () = waiting.reply.closed() => WaitEnd::Gone,
            }
        }
        None => std::future::pending().await,
    }
}

pub struct Daemon<T: Transport, R: Runner, F: ClientFactory> {
    transport: Arc<T>,
    runner: R,
    factory: Arc<F>,
    store: Store,
    config_path: PathBuf,
    config_stamp: Option<SystemTime>,
    config_error: Option<String>,
    drawings_dir: PathBuf,
    drawings_stamp: Option<SystemTime>,
    /// The taught drawings' names (for the config notes).
    drawing_names: Vec<String>,
    /// The hand whose calibration is in the controls.
    options_hand: Option<Hand>,
    /// The teaching step waiting for the band.
    waiting: Option<Waiting>,
    controls: Controls<String>,
    /// The loaded config (hooks, dial commands, profiles).
    config: Config,
    /// The IPC dial choice, while `controls.dial_overridden()`.
    dial_override: DialTarget,
    /// Each profile's own index dial (from the config), for status.
    index_dials: Vec<DialTarget>,
    clock: Clock,
    status: watch::Sender<Status>,
    semg: broadcast::Sender<Vec<u8>>,
    wants_connection: bool,
    retries: u32,
    reconnect_at: Option<f64>,
    link: Option<ActiveLink>,
    state: LinkState,
    error: Option<String>,
    pairing: Option<PairingTask>,
    pairing_stage: Option<String>,
    /// Why the last pairing failed; outlives reconnects (cleared by the next
    /// pairing, a successful one, or `Forget`).
    pairing_error: Option<String>,
    last_pair: Option<PairResult>,
    /// Reconnect the already-paired band if this pairing fails.
    resume_after_pairing: bool,
    progress_tx: mpsc::UnboundedSender<String>,
    progress_rx: mpsc::UnboundedReceiver<String>,
    semg_wanted: bool,
    semg_on: bool,
    /// Since when sEMG has been on (or wanted) with no viewer subscribed.
    semg_unwatched_since: Option<f64>,
    band_name: Option<String>,
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
}

async fn next_event(link: &mut Option<ActiveLink>) -> Option<LinkEvent> {
    match link {
        Some(link) => link.events.recv().await,
        None => std::future::pending().await,
    }
}

async fn join<T>(task: &mut Option<JoinHandle<T>>) -> Result<T, String> {
    match task {
        Some(task) => task.await.map_err(|error| error.to_string()),
        None => std::future::pending().await,
    }
}

impl<T: Transport, R: Runner, F: ClientFactory> Daemon<T, R, F> {
    pub fn new(
        transport: Arc<T>,
        runner: R,
        factory: F,
        store: Store,
        config_path: PathBuf,
        drawings_dir: PathBuf,
    ) -> Self {
        let (config, config_error) = match Config::load(&config_path) {
            Ok(config) => (config, None),
            Err(error) => (Config::default(), Some(format!("config: {error}"))),
        };
        let config_stamp = modified(&config_path);
        let wants_connection = config.band.autostart;
        let index_dials = config.index_dials().to_vec();
        let options_hand = configured_hand(&config);
        let mut controls = Controls::new(config.options(), config.profiles().clone());
        let (palm, drags) = calibration_for(&store, options_hand);
        controls.set_calibration(palm, drags);
        log_notes(controls.profiles());
        let band_name = store.load_band().ok().flatten().map(|band| band.name);
        let (status, _) = watch::channel(Status::default());
        let (semg, _) = broadcast::channel(256);
        let (progress_tx, progress_rx) = mpsc::unbounded_channel();
        let mut daemon = Self {
            transport,
            runner,
            factory: Arc::new(factory),
            store,
            config_path,
            config_stamp,
            config_error,
            drawings_dir,
            drawings_stamp: None,
            drawing_names: Vec::new(),
            options_hand,
            waiting: None,
            controls,
            config,
            dial_override: DialTarget::Off,
            index_dials,
            clock: Clock::new(),
            status,
            semg,
            wants_connection,
            retries: 0,
            reconnect_at: wants_connection.then_some(0.0),
            link: None,
            state: LinkState::Idle,
            error: None,
            pairing: None,
            pairing_stage: None,
            pairing_error: None,
            last_pair: None,
            resume_after_pairing: false,
            progress_tx,
            progress_rx,
            semg_wanted: false,
            semg_on: false,
            semg_unwatched_since: None,
            band_name,
        };
        daemon.reload_drawings(true);
        daemon.publish();
        daemon
    }

    pub fn status(&self) -> watch::Receiver<Status> {
        self.status.subscribe()
    }

    pub fn semg(&self) -> broadcast::Sender<Vec<u8>> {
        self.semg.clone()
    }

    pub async fn run(
        mut self,
        mut calls: mpsc::Receiver<Call>,
        shutdown: impl Future<Output = ()>,
    ) {
        tokio::pin!(shutdown);
        let mut ticker = tokio::time::interval(TICK);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = &mut shutdown => break,
                call = calls.recv() => match call {
                    Some((request, reply)) => self.handle(request, reply).await,
                    None => break,
                },
                event = next_event(&mut self.link) => match event {
                    Some(event) => self.on_link_event(event),
                    None => self.on_link_closed().await,
                },
                result = join(&mut self.pairing) => {
                    self.pairing = None;
                    self.on_pairing_end(result);
                }
                Some(stage) = self.progress_rx.recv() => {
                    if self.pairing.is_some() {
                        self.pairing_stage = Some(stage);
                    }
                }
                _ = ticker.tick() => self.on_tick(),
                _ = until(self.controls.next_deadline(), self.clock) => {
                    let now = self.clock.now();
                    let outputs = self.controls.poll(now);
                    self.apply(outputs, true);
                }
                end = wait_ends(&mut self.waiting, self.clock) => match end {
                    WaitEnd::TimedOut => self.end_wait(Wait::timed_out),
                    WaitEnd::Gone => self.end_wait(|_| Done::Failed("the client went away".into())),
                },
            }
            self.publish();
        }
        self.stop_link().await;
        if let Some(pairing) = self.pairing.take() {
            pairing.abort();
        }
    }

    fn publish(&self) {
        let controls = self.controls.status();
        let mut status = Status {
            paused: controls.paused,
            battery: controls.battery,
            charging: controls.charging,
            hand: controls.hand.map(|hand| match hand {
                Hand::Left => "left".to_owned(),
                Hand::Right => "right".to_owned(),
            }),
            dial: self.dial_target(),
            last_gesture: controls.last_gesture.map(str::to_owned),
            last_action: controls.last_action.clone(),
            gestures: controls.gestures,
            profile: (self.controls.profiles().list.len() > 1).then(|| {
                self.controls.profiles().list[self.controls.active()]
                    .name
                    .clone()
            }),
            ..Status::default()
        };
        status.state = self.state;
        status.error = self.error.clone();
        status.pairing_error = self.pairing_error.clone();
        status.config_error = self.config_error.clone();
        status.last_pair = self.last_pair.clone();
        // Whole seconds, so subscribers see one change per second, not per tick.
        status.reconnect_in = match self.reconnect_at {
            Some(at) if self.wants_connection && self.link.is_none() && self.pairing.is_none() => {
                Some((at - self.clock.now()).max(0.0).ceil() as u32)
            }
            _ => None,
        };
        status.pairing = self.pairing_stage.clone();
        status.semg = self.semg_on;
        status.band = self.band_name.clone();
        self.status.send_if_modified(|current| {
            if *current == status {
                false
            } else {
                *current = status;
                true
            }
        });
    }

    /// Never waits: the link blocks its read loop on sending us events, so
    /// awaiting room in its command queue here could deadlock the two.
    fn send_link(&self, command: LinkCommand) -> bool {
        match &self.link {
            Some(link) => match link.commands.try_send(command) {
                Ok(()) => true,
                Err(error) => {
                    tracing::warn!("band link busy; dropped a command: {error}");
                    false
                }
            },
            None => false,
        }
    }

    /// Do what the controls asked. `from_band`: caused by a band gesture, not a request
    /// (`on_toggle` only runs for those, as before).
    fn apply(&mut self, outputs: Vec<Output<String>>, from_band: bool) {
        for output in outputs {
            if let Some(done) = self.waiting.as_mut().and_then(|waiting| waiting.wait.on_output(&output)) {
                self.end_wait(|_| done);
            }
            match output {
                Output::Run(command) => {
                    tracing::info!(%command, "action");
                    self.runner.run(command);
                }
                Output::SetHand(hand) => {
                    self.send_link(LinkCommand::SetHand(hand));
                }
                Output::Gesture(gesture) => {
                    tracing::debug!(
                        gesture = gesture.label(),
                        enabled = self.controls.enabled(),
                        "recognized"
                    );
                }
                Output::Controls { enabled } => {
                    tracing::info!(enabled, "controls");
                    let hook = self.config.band.on_toggle.trim();
                    if from_band && !hook.is_empty() {
                        // Exported, not a prefix assignment: the hook's own `$AIR_GESTURES_ENABLED`
                        // expansions must see it.
                        self.runner.run(format!(
                            "export AIR_GESTURES_ENABLED={}; {hook}",
                            u8::from(enabled)
                        ));
                    }
                }
                Output::Profile { index, .. } => {
                    let name = self.controls.profiles().list[index].name.clone();
                    tracing::info!(profile = %name, "profile");
                    let hook = self.config.band.on_profile.trim();
                    if !hook.is_empty() {
                        // Names are checked when the config loads: no quotes inside.
                        self.runner
                            .run(format!("export AIR_GESTURES_PROFILE='{name}'; {hook}"));
                    }
                }
                Output::ProfileHeld { app } => {
                    tracing::debug!(%app, "profile held by the app in front")
                }
                Output::Practiced(_) => {}
                Output::DrawPoint { stroke, x, y } => tracing::trace!(stroke, x, y, "draw point"),
                Output::Pen { down } => tracing::trace!(down, "pen"),
                Output::Drawn { name, distance } => {
                    tracing::info!(drawing = %name, distance, "drawn");
                    self.run_draw_hook(&name);
                }
                Output::NotDrawn => {
                    tracing::info!("drawing not recognised");
                    self.run_draw_hook("");
                }
                other @ (Output::DrawStarted
                | Output::DrawFinished { .. }
                | Output::DrawCancelled
                | Output::DragSample { .. }) => tracing::debug!(?other, "controls"),
            }
        }
    }

    /// What the index dial turns now: the IPC choice, else the active profile's own.
    fn dial_target(&self) -> DialTarget {
        if self.controls.dial_overridden() {
            self.dial_override
        } else {
            self.index_dials.get(self.controls.active()).copied().unwrap_or(DialTarget::Off)
        }
    }

    fn override_dial(&mut self, target: DialTarget) {
        self.dial_override = target;
        self.controls.set_dial(self.config.dial.dial(target));
    }

    /// The hand whose calibration applies: the band's, else the configured one.
    fn hand_in_use(&self) -> Option<Hand> {
        self.controls.status().hand.or_else(|| configured_hand(&self.config))
    }

    /// The hand in use changed (or, `force`, what's taught for it): its calibration into the
    /// controls. `set_calibration` keeps the hand (an IPC choice too) and writes nothing.
    fn refresh_calibration(&mut self, force: bool) {
        let hand = self.hand_in_use();
        if !force && hand == self.options_hand {
            return;
        }
        self.options_hand = hand;
        let (palm, drags) = calibration_for(&self.store, hand);
        if hand == Some(Hand::Left) && palm.is_none() && uses_palm(self.controls.profiles()) {
            tracing::info!("the left hand has no palm calibration: every gesture counts as palm down until `air-gestures calibrate`");
        }
        self.controls.set_calibration(palm, drags);
    }

    /// The taught drawings into the controls, when the directory changed (or `force`).
    fn reload_drawings(&mut self, force: bool) {
        let stamp = drawings::stamp(&self.drawings_dir);
        if !force && stamp == self.drawings_stamp {
            return;
        }
        self.drawings_stamp = stamp;
        let (drawings, problems) = drawings::load(&self.drawings_dir);
        for problem in problems {
            tracing::warn!("drawings: {problem}");
        }
        tracing::info!(count = drawings.len(), "drawings loaded");
        self.drawing_names = drawings.iter().map(|d| d.name.clone()).collect();
        self.controls.set_library(Library::new(&drawings));
        self.log_missing_drawings();
    }

    /// Drawings with an action but no template.
    fn log_missing_drawings(&self) {
        let names: Vec<&str> = self.drawing_names.iter().map(String::as_str).collect();
        for note in crate::config::missing_drawings(self.controls.profiles(), &names) {
            tracing::warn!("config: {}", note.text);
        }
    }

    /// Start a teaching step (`Request::teach_step`); its answer goes to `reply` when it ends.
    fn start_wait(&mut self, request: &Request, reply: oneshot::Sender<Response>) {
        let Some((step, seconds)) = request.teach_step() else {
            let _ = reply.send(Response::error("not a teaching step"));
            return;
        };
        if self.state != LinkState::Connected {
            let _ = reply.send(Response::error(CONNECT_FIRST));
            return;
        }
        if self.waiting.as_ref().is_some_and(|waiting| waiting.reply.is_closed()) {
            // Its client is gone (the loop hasn't noticed yet): this one goes ahead.
            self.end_wait(|_| Done::Failed("the client went away".into()));
        }
        if self.waiting.is_some() {
            let _ = reply.send(Response::error("another teaching step is running; wait for it to finish"));
            return;
        }
        let now = self.clock.now();
        let outputs = step.enter(&mut self.controls, now);
        self.apply(outputs, false);
        self.waiting = Some(Waiting { wait: Wait::new(step, now, seconds), reply });
    }

    /// End the waiting step (if any) with what `done` makes of it: teaching off again, and
    /// the answer (lost if the client went).
    fn end_wait(&mut self, done: impl FnOnce(Wait) -> Done) {
        let Some(Waiting { wait, reply }) = self.waiting.take() else { return };
        let outputs = wait.step().leave(&mut self.controls);
        let done = done(wait);
        self.apply(outputs, false);
        self.publish();
        let _ = reply.send(match done {
            Done::Taught(taught) => Response::taught(self.status.borrow().clone(), taught),
            Done::Retry(why) => Response::error_code(why, crate::ipc::CODE_TRY_AGAIN),
            Done::Failed(why) => Response::error(why),
        });
    }

    /// Change what's taught for the paired band and the hand in use, and apply it.
    fn save_taught(&mut self, change: impl FnOnce(&mut HandCalibration)) -> Response {
        let (Some(hand), Ok(Some(band))) = (self.hand_in_use(), self.store.load_band()) else {
            return Response::error(CONNECT_FIRST);
        };
        let mut calibration = self.store.load_calibration(&band.address, hand).unwrap_or_else(|error| {
            tracing::warn!("calibration: {error}; starting afresh");
            HandCalibration::default()
        });
        change(&mut calibration);
        if let Err(error) = self.store.save_calibration(&band.address, hand, &calibration) {
            return Response::error(format!("state: {error}"));
        }
        self.refresh_calibration(true);
        self.publish();
        Response::ok(self.status.borrow().clone())
    }

    fn run_draw_hook(&self, name: &str) {
        let hook = self.config.band.on_draw.trim();
        if !hook.is_empty() {
            // Names are checked (config and drawings): no quotes inside.
            self.runner.run(format!("export AIR_GESTURES_DRAWING='{name}'; {hook}"));
        }
    }

    fn on_link_event(&mut self, event: LinkEvent) {
        let event = match event {
            LinkEvent::Refused(message) => {
                tracing::info!("band: {message}");
                return;
            }
            LinkEvent::Band(event) => event,
        };
        match &event {
            Event::Connected => {
                self.state = LinkState::Connected;
                self.retries = 0;
                self.error = None;
                tracing::info!("band connected");
                // The band accepted the key from an unconfirmed claim: it is ours now.
                if let Some(link) = &mut self.link
                    && link.pending
                {
                    link.pending = false;
                    if let Err(error) = self.store.promote_pending() {
                        tracing::warn!("could not confirm the pending owner key: {error}");
                    }
                }
            }
            Event::RawEmgFrame(frame) => {
                let _ = self.semg.send(frame.clone());
            }
            Event::RawEmgState(on) => self.semg_on = *on,
            Event::RawEmgFailure(message) | Event::HandednessFailure(message) => {
                tracing::warn!("band: {message}")
            }
            _ => {}
        }
        if let Some(waiting) = &mut self.waiting {
            waiting.wait.on_event(&event);
        }
        let now = self.clock.now();
        let outputs = self.controls.on_event(&event, now);
        self.apply(outputs, true);
        self.refresh_calibration(false);
    }

    async fn on_link_closed(&mut self) {
        let Some(link) = self.link.take() else { return };
        let used_pending = link.pending;
        let outcome = link.task.await.unwrap_or_else(|error| LinkOutcome {
            end: LinkEnd::Failed(error.to_string()),
            enrollment: None,
            scheme_guess: None,
            authenticated: false,
        });
        self.controls.on_disconnect();
        self.end_wait(|_| Done::Failed("the band disconnected".into()));
        self.semg_on = false;
        self.state = LinkState::Idle;
        match &outcome.end {
            // Only 0x1043 proves the unconfirmed claim never committed: drop
            // that key and retry with the confirmed one right away.
            LinkEnd::IdentityRejected {
                code: DIFFERENT_KEY,
                ..
            } if used_pending && self.store.load_identity().ok().flatten().is_some() => {
                match self.store.discard_pending() {
                    Ok(()) => self.reconnect_at = Some(self.clock.now()),
                    Err(error) => {
                        self.error = Some(format!(
                            "could not drop the rejected pending owner key: {error}"
                        ));
                        self.schedule_reconnect();
                    }
                }
                return;
            }
            // 0x1043 on the confirmed key (or on a pending one with nothing to
            // fall back to): the band belongs to another key; claiming it again
            // is the only way back.
            LinkEnd::IdentityRejected {
                code: DIFFERENT_KEY,
                message,
            } => {
                self.error = Some(format!(
                    "{message} Run `air-gestures forget`, factory reset the band ({FACTORY_RESET_HINT}), then `air-gestures pair`."
                ));
                self.wants_connection = false;
            }
            // Any other code may be transient: keep the key and retry with backoff.
            LinkEnd::IdentityRejected { message, .. } => {
                tracing::warn!("band rejected the owner key for now: {message}");
                self.error = Some(message.clone());
            }
            LinkEnd::Closed => {}
            other => {
                tracing::warn!("band link ended: {other}");
                self.error = Some(other.to_string());
            }
        }
        if self.wants_connection {
            self.schedule_reconnect();
        }
    }

    fn schedule_reconnect(&mut self) {
        self.retries += 1;
        let delay = f64::from((2 * self.retries).min(10));
        self.reconnect_at = Some(self.clock.now() + delay);
    }

    fn start_link(&mut self) {
        let band = match self.store.load_band() {
            Ok(Some(band)) => band,
            Ok(None) => {
                self.error = Some("no band paired yet — run `air-gestures pair`".into());
                self.wants_connection = false;
                return;
            }
            Err(error) => {
                self.error = Some(format!("state: {error}"));
                self.wants_connection = false;
                return;
            }
        };
        let (identity, pending) = match self.store.connect_identity() {
            Ok(Some((identity, source))) => (Some(identity), source == IdentitySource::Pending),
            Ok(None) => (None, false),
            Err(error) => {
                self.error = Some(format!("state: {error}"));
                self.wants_connection = false;
                return;
            }
        };
        self.band_name = Some(band.name.clone());
        self.state = LinkState::Connecting;
        let (commands, command_rx) = mpsc::channel(16);
        let (event_tx, events) = mpsc::channel(256);
        let transport = self.transport.clone();
        let session = BandSession::new(identity, None, self.semg_wanted)
            .with_scheme_guess(band.scheme_guess.unwrap_or(0));
        let clock = self.clock;
        let task = tokio::spawn(async move {
            match transport.open(&band.address).await {
                Ok(io) => run_link(io, session, clock, command_rx, event_tx, |_| Ok(())).await,
                Err(error) => LinkOutcome {
                    end: LinkEnd::Failed(format!("could not reach {}: {error}", band.name)),
                    enrollment: None,
                    scheme_guess: None,
                    authenticated: false,
                },
            }
        });
        self.link = Some(ActiveLink {
            commands,
            events,
            task,
            pending,
        });
    }

    async fn stop_link(&mut self) {
        let Some(link) = self.link.take() else { return };
        self.controls.on_disconnect();
        self.end_wait(|_| Done::Failed("the band disconnected".into()));
        self.semg_on = false;
        self.state = LinkState::Idle;
        self.publish();
        // Dropping the command sender stops the link too, if its queue is full.
        let _ = link.commands.try_send(LinkCommand::Stop);
        drop(link.commands);
        drop(link.events);
        let mut task = link.task;
        if tokio::time::timeout(Duration::from_secs(5), &mut task)
            .await
            .is_err()
        {
            task.abort();
        }
    }

    fn on_tick(&mut self) {
        let now = self.clock.now();
        if self.wants_connection
            && self.link.is_none()
            && self.pairing.is_none()
            && self.reconnect_at.is_some_and(|at| now >= at)
        {
            self.reconnect_at = None;
            self.start_link();
        }
        self.stop_unwatched_semg(now);
        self.reload_config();
        self.reload_drawings(false);
    }

    /// sEMG streaming is tied to viewers: with none attached for
    /// `SEMG_IDLE` seconds, turn it off (and don't turn it on at reconnect).
    fn stop_unwatched_semg(&mut self, now: f64) {
        if !(self.semg_on || self.semg_wanted) || self.semg.receiver_count() > 0 {
            self.semg_unwatched_since = None;
            return;
        }
        let since = *self.semg_unwatched_since.get_or_insert(now);
        if now - since >= SEMG_IDLE {
            tracing::info!("no sEMG viewer attached; turning raw sEMG off");
            self.semg_wanted = false;
            self.semg_unwatched_since = None;
            self.send_link(LinkCommand::RawEmg(false));
        }
    }

    fn reload_config(&mut self) {
        let stamp = modified(&self.config_path);
        if stamp == self.config_stamp {
            return;
        }
        self.config_stamp = stamp;
        match Config::load(&self.config_path) {
            Ok(config) => {
                self.config_error = None;
                tracing::info!("config reloaded");
                let active = self.controls.active();
                let before = self.index_dials.get(active).copied();
                self.config = config;
                self.index_dials = self.config.index_dials().to_vec();
                let mut outputs = self.controls.set_options(self.config.options());
                // Those options carry no calibration: put it back.
                self.refresh_calibration(true);
                outputs.extend(self.controls.set_profiles(self.config.profiles().clone()));
                if self.controls.dial_overridden() {
                    // A changed dial in the file wins over a runtime choice, as before.
                    if self.index_dials.get(self.controls.active()).copied() != before {
                        self.controls.clear_dial();
                    } else {
                        self.override_dial(self.dial_override);
                    }
                }
                log_notes(self.controls.profiles());
                self.log_missing_drawings();
                self.apply(outputs, false);
            }
            Err(error) => {
                tracing::warn!("config: {error}");
                self.config_error = Some(format!("config: {error}"));
            }
        }
    }

    fn on_pairing_end(&mut self, result: Result<Result<BandRecord, PairFailure>, String>) {
        // Stages still queued from the finished pairing must not show up later.
        while self.progress_rx.try_recv().is_ok() {}
        self.pairing_stage = None;
        self.state = LinkState::Idle;
        // The pairing's own outcome lives in pairing_error/last_pair; any
        // link error from before it started is stale now, whatever the result.
        self.error = None;
        let mut needs_sign_in = false;
        let failure = match result {
            Ok(Ok(record)) => {
                tracing::info!("paired with {}", record.name);
                self.last_pair = Some(PairResult {
                    ok: true,
                    message: format!("paired with {}", record.name),
                    needs_sign_in: false,
                });
                self.band_name = Some(record.name);
                self.pairing_error = None;
                // The settling reconnect: connect again as the new owner.
                self.wants_connection = true;
                self.retries = 0;
                self.reconnect_at = Some(self.clock.now());
                return;
            }
            Ok(Err(PairFailure::SessionExpired(message))) => {
                let _ = self.store.delete_meta_session();
                needs_sign_in = true;
                message
            }
            Ok(Err(failure)) => failure.to_string(),
            Err(error) => format!("the pairing crashed: {error}"),
        };
        tracing::warn!("pairing failed: {failure}");
        self.last_pair = Some(PairResult {
            ok: false,
            message: failure.clone(),
            needs_sign_in,
        });
        self.pairing_error = Some(failure);
        // Go back to the band we had (or the one a possibly committed claim left pending).
        let has_pending = self.store.load_pending().ok().flatten().is_some();
        let has_band = self.store.load_band().ok().flatten();
        if (self.resume_after_pairing || has_pending)
            && let Some(band) = has_band
        {
            self.band_name = Some(band.name);
            self.wants_connection = true;
            self.retries = 0;
            self.reconnect_at = Some(self.clock.now());
        }
    }

    async fn start_pairing(&mut self, address: Option<String>) -> Response {
        // Pairing strips the Bluetooth bond of the band it targets: never risk
        // that on the band we own.
        match self.store.load_band() {
            Ok(None) => {}
            Ok(Some(_)) => return Response::error(ALREADY_PAIRED),
            Err(error) => return Response::error(format!("state: {error}")),
        }
        let session = match self.store.load_meta_session() {
            Ok(session) => session,
            Err(error) => return Response::error(format!("state: {error}")),
        };
        let client = match self.factory.client(session) {
            Ok(client) => client,
            Err(Some(message)) => return Response::error(message),
            Err(None) => {
                return Response::error_code(
                    "sign in to Meta first (`air-gestures pair` opens the sign-in window)",
                    crate::ipc::CODE_NEEDS_SIGN_IN,
                );
            }
        };
        self.resume_after_pairing = self.wants_connection || self.link.is_some();
        self.wants_connection = false;
        self.reconnect_at = None;
        self.stop_link().await;
        self.state = LinkState::Pairing;
        self.error = None;
        self.pairing_error = None;
        self.last_pair = None;
        self.pairing_stage = Some("starting".into());
        let transport = self.transport.clone();
        let store = self.store.clone();
        let clock = self.clock;
        let progress = self.progress_tx.clone();
        self.pairing = Some(tokio::spawn(async move {
            pair_band(
                &*transport,
                &client,
                &store,
                address.as_deref(),
                clock,
                move |stage: &str| {
                    let _ = progress.send(stage.to_owned());
                },
            )
            .await
        }));
        self.publish();
        Response::message(
            self.status.borrow().clone(),
            "put the band in pairing mode (hold its button for 3 seconds until it flashes)",
        )
    }

    /// Answer a call: now, or (a teaching step) when it ends.
    async fn handle(&mut self, request: Request, reply: oneshot::Sender<Response>) {
        let now = self.clock.now();
        let response = 'answer: {
            match request {
                Request::Status => {}
                Request::Connect => {
                    if self.pairing.is_some() {
                        break 'answer Response::error("a pairing is running");
                    }
                    self.wants_connection = true;
                    self.retries = 0;
                    self.error = None;
                    if self.link.is_none() {
                        self.reconnect_at = None;
                        self.start_link();
                        if self.link.is_none()
                            && let Some(error) = &self.error
                        {
                            let response = Response::error(error.clone());
                            self.publish();
                            break 'answer response;
                        }
                    }
                }
                Request::Disconnect => {
                    if self.pairing.is_some() {
                        break 'answer Response::error(PAIRING_RUNNING);
                    }
                    self.wants_connection = false;
                    self.error = None;
                    self.reconnect_at = None;
                    self.stop_link().await;
                    // Let go of the band entirely, so a phone can connect to it.
                    if let Ok(Some(band)) = self.store.load_band() {
                        self.transport.release(&band.address).await;
                    }
                }
                Request::Pause => {
                    let outputs = self.controls.pause();
                    self.apply(outputs, false);
                }
                Request::Resume => {
                    let outputs = self.controls.resume(now);
                    self.apply(outputs, false);
                }
                Request::TogglePause => {
                    let outputs = self.controls.toggle(now);
                    self.apply(outputs, false);
                }
                Request::Dial { target } => self.override_dial(target),
                Request::CycleDial => self.override_dial(self.dial_target().next()),
                Request::Profile { to } => {
                    let count = self.controls.profiles().list.len();
                    let active = self.controls.active();
                    let index = match to.as_str() {
                        "next" => (active + 1) % count,
                        "previous" => (active + count - 1) % count,
                        name => {
                            let list = &self.controls.profiles().list;
                            match list.iter().position(|p| p.name == name) {
                                Some(index) => index,
                                None => {
                                    let names: Vec<&str> =
                                        list.iter().map(|p| p.name.as_str()).collect();
                                    break 'answer Response::error(format!(
                                        "no profile \"{name}\" (profiles: {})",
                                        names.join(", ")
                                    ));
                                }
                            }
                        }
                    };
                    let outputs = self.controls.pick(index);
                    self.apply(outputs, false);
                }
                Request::Hand { hand } => {
                    if self.state != LinkState::Connected {
                        break 'answer Response::error(CONNECT_FIRST);
                    }
                    let hand = match hand {
                        HandChoice::Left => Hand::Left,
                        HandChoice::Right => Hand::Right,
                    };
                    // Through the controls, so the config's hand doesn't undo the choice.
                    let outputs = self.controls.request_hand(hand);
                    self.apply(outputs, false);
                }
                Request::Semg { on } => {
                    self.semg_wanted = on;
                    self.send_link(LinkCommand::RawEmg(on));
                }
                Request::Pair { address } => {
                    if self.pairing.is_some() {
                        break 'answer Response::error("a pairing is already running");
                    }
                    break 'answer self.start_pairing(address).await;
                }
                Request::Forget => {
                    if self.pairing.is_some() {
                        break 'answer Response::error(PAIRING_RUNNING);
                    }
                    self.wants_connection = false;
                    self.reconnect_at = None;
                    self.stop_link().await;
                    if let Ok(Some(band)) = self.store.load_band()
                        && let Err(error) = self.transport.forget(&band.address).await
                    {
                        tracing::warn!("could not remove the Bluetooth bond: {error}");
                    }
                    if let Err(error) = self.store.forget() {
                        break 'answer Response::error(format!("state: {error}"));
                    }
                    self.band_name = None;
                    self.error = None;
                    self.pairing_error = None;
                    self.last_pair = None;
                    self.publish();
                    break 'answer Response::message(
                        self.status.borrow().clone(),
                        format!(
                            "forgot the band. factory reset it too ({FACTORY_RESET_HINT}) before pairing it anywhere else."
                        ),
                    );
                }
                Request::Subscribe | Request::SubscribeSemg => {
                    break 'answer Response::error("subscriptions are handled by the control socket");
                }
                Request::PalmRolls { .. } | Request::TeachDrag { .. } | Request::DrawTry { .. } => {
                    return self.start_wait(&request, reply);
                }
                Request::SetPalm { palm } => {
                    if let Some(p) = palm
                        && let Err(error) = PalmCalibration::from_rolls(&[p.down_roll], &[p.up_roll])
                    {
                        break 'answer Response::error(crate::teach::calibration_error_text(error));
                    }
                    break 'answer self.save_taught(|calibration| calibration.palm = palm);
                }
                Request::SetDrags { drags } => {
                    let stored = match drags {
                        None => None,
                        Some(taught) => match Prototypes::taught(taught.turn, taught.drags, self.controls.prototypes().palm_down_roll) {
                            Ok(prototypes) => StoredDrags::from_prototypes(&prototypes),
                            Err(error) => break 'answer Response::error(crate::teach::taught_error_text(error)),
                        },
                    };
                    break 'answer self.save_taught(|calibration| calibration.drags = stored);
                }
                Request::Palm => {
                    if self.state != LinkState::Connected {
                        break 'answer Response::error(CONNECT_FIRST);
                    }
                    let palm = self.controls.palm(now).key().to_owned();
                    self.publish();
                    break 'answer Response::taught(self.status.borrow().clone(), Taught::Palm { palm });
                }
            }

            self.publish();
            Response::ok(self.status.borrow().clone())
        };
        let _ = reply.send(response);
    }
}

fn log_notes(profiles: &Profiles<String>) {
    for note in crate::config::notes(profiles) {
        match note.level {
            Level::Info => tracing::info!("config: {}", note.text),
            Level::Warn => tracing::warn!("config: {}", note.text),
            Level::Error => tracing::error!("config: {}", note.text),
        }
    }
}

/// The configured hand, before the band says.
fn configured_hand(config: &Config) -> Option<Hand> {
    match config.band.hand {
        HandSetting::Left => Some(Hand::Left),
        HandSetting::Right => Some(Hand::Right),
        HandSetting::Band => None,
    }
}

/// What was taught on the paired band for `hand`, for `Controls::set_calibration`
/// (`None`s: the core's defaults for that hand).
fn calibration_for(store: &Store, hand: Option<Hand>) -> (Option<PalmCalibration>, Option<Prototypes>) {
    let (Some(hand), Ok(Some(band))) = (hand, store.load_band()) else { return (None, None) };
    match store.load_calibration(&band.address, hand) {
        Ok(calibration) => {
            let (palm, drags, problems) = calibration.apply();
            for problem in problems {
                tracing::warn!("calibration: {problem}");
            }
            (palm, drags)
        }
        Err(error) => {
            tracing::warn!("calibration: {error}");
            (None, None)
        }
    }
}

/// Some gesture or band control depends on the palm position.
fn uses_palm(profiles: &Profiles<String>) -> bool {
    profiles.list.iter().any(|p| p.gestures.values().any(|b| !b.overrides.is_empty()))
        || Builtin::ALL.into_iter().any(|b| profiles.band.control(b).is_some_and(|c| c.palm != PalmSet::All))
}

/// Sleeps until `deadline` on `clock`; never finishes without one.
async fn until(deadline: Option<f64>, clock: Clock) {
    match deadline {
        Some(at) => tokio::time::sleep(Duration::from_secs_f64((at - clock.now()).max(0.0))).await,
        None => std::future::pending().await,
    }
}
