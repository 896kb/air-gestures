//! The supervisor: keeps one link to the paired band (reconnecting with
//! backoff), routes its events through the controller to the runner, reloads
//! the config, runs pairings and answers control-socket calls.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use band_core::ceremony::FACTORY_RESET_HINT;
use band_core::events::{Event, Hand};
use band_core::session::BandSession;
use tokio::sync::{broadcast, mpsc, watch};
use tokio::task::JoinHandle;

use crate::actions::Runner;
use crate::clock::Clock;
use crate::config::Config;
use crate::controller::{Command, Controller};
use crate::ipc::{Call, HandChoice, Request, Response};
use crate::link::{LinkCommand, LinkEnd, LinkEvent, LinkOutcome, run_link};
use crate::meta::ClientFactory;
use crate::pairing::{PairFailure, pair_band};
use crate::status::{LinkState, Status};
use crate::store::{BandRecord, IdentitySource, Store};
use crate::transport::Transport;

const TICK: Duration = Duration::from_millis(250);
/// Raw sEMG is turned off after this long without a viewer attached.
const SEMG_IDLE: f64 = 3.0;
/// Identity result code: the band is enrolled to a different key.
const DIFFERENT_KEY: u32 = 0x1043;
const ALREADY_PAIRED: &str =
    "a band is already paired — forget it first (`kinesisctl forget`, then factory reset the band)";
const PAIRING_RUNNING: &str = "a pairing is running; wait for it to finish";

struct ActiveLink {
    commands: mpsc::Sender<LinkCommand>,
    events: mpsc::Receiver<LinkEvent>,
    task: JoinHandle<LinkOutcome>,
    /// The link authenticates with the pending key from an unconfirmed claim.
    pending: bool,
}

type PairingTask = JoinHandle<Result<BandRecord, PairFailure>>;

pub struct Daemon<T: Transport, R: Runner, F: ClientFactory> {
    transport: Arc<T>,
    runner: R,
    factory: Arc<F>,
    store: Store,
    config_path: PathBuf,
    config_stamp: Option<SystemTime>,
    config_error: Option<String>,
    controller: Controller,
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
    ) -> Self {
        let (config, config_error) = match Config::load(&config_path) {
            Ok(config) => (config, None),
            Err(error) => (Config::default(), Some(format!("config: {error}"))),
        };
        let config_stamp = modified(&config_path);
        let wants_connection = config.band.autostart;
        let band_name = store.load_band().ok().flatten().map(|band| band.name);
        let (status, _) = watch::channel(Status::default());
        let (semg, _) = broadcast::channel(256);
        let (progress_tx, progress_rx) = mpsc::unbounded_channel();
        let daemon = Self {
            transport,
            runner,
            factory: Arc::new(factory),
            store,
            config_path,
            config_stamp,
            config_error,
            controller: Controller::new(config),
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
            resume_after_pairing: false,
            progress_tx,
            progress_rx,
            semg_wanted: false,
            semg_on: false,
            semg_unwatched_since: None,
            band_name,
        };
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
                    Some((request, reply)) => {
                        let response = self.handle(request).await;
                        let _ = reply.send(response);
                    }
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
            }
            self.publish();
        }
        self.stop_link().await;
        if let Some(pairing) = self.pairing.take() {
            pairing.abort();
        }
    }

    fn publish(&self) {
        let mut status = self.controller.status().clone();
        status.state = self.state;
        status.error = self
            .error
            .clone()
            .or_else(|| self.pairing_error.clone())
            .or_else(|| self.config_error.clone());
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

    fn apply(&mut self, command: Command) {
        match command {
            Command::Run(command) => {
                tracing::info!(%command, "action");
                self.runner.run(command);
            }
            Command::SetHand(hand) => {
                self.send_link(LinkCommand::SetHand(hand));
            }
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
        let now = self.clock.now();
        for command in self.controller.on_event(&event, now) {
            self.apply(command);
        }
    }

    async fn on_link_closed(&mut self) {
        let Some(link) = self.link.take() else { return };
        let used_pending = link.pending;
        let outcome = link.task.await.unwrap_or_else(|error| LinkOutcome {
            end: LinkEnd::Failed(error.to_string()),
            enrollment: None,
        });
        self.controller.on_disconnect();
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
                    "{message} Run `kinesisctl pair` to claim the band again."
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
                self.error = Some("no band paired yet — run `kinesisctl pair`".into());
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
        let session = BandSession::new(identity, None, self.semg_wanted);
        let clock = self.clock;
        let task = tokio::spawn(async move {
            match transport.open(&band.address).await {
                Ok(io) => run_link(io, session, clock, command_rx, event_tx, |_| Ok(())).await,
                Err(error) => LinkOutcome {
                    end: LinkEnd::Failed(format!("could not reach {}: {error}", band.name)),
                    enrollment: None,
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
        self.controller.on_disconnect();
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
                for command in self.controller.set_config(config) {
                    self.apply(command);
                }
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
        let failure = match result {
            Ok(Ok(record)) => {
                tracing::info!("paired with {}", record.name);
                self.band_name = Some(record.name);
                self.error = None;
                self.pairing_error = None;
                // The settling reconnect: connect again as the new owner.
                self.wants_connection = true;
                self.retries = 0;
                self.reconnect_at = Some(self.clock.now());
                return;
            }
            Ok(Err(PairFailure::SessionExpired(message))) => {
                let _ = self.store.delete_meta_session();
                message
            }
            Ok(Err(failure)) => failure.to_string(),
            Err(error) => format!("the pairing crashed: {error}"),
        };
        tracing::warn!("pairing failed: {failure}");
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
                return Response::error(
                    "sign in to Meta first (`kinesisctl pair` opens the sign-in window)",
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

    async fn handle(&mut self, request: Request) -> Response {
        let now = self.clock.now();
        match request {
            Request::Status => {}
            Request::Connect => {
                if self.pairing.is_some() {
                    return Response::error("a pairing is running");
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
                        return response;
                    }
                }
            }
            Request::Disconnect => {
                if self.pairing.is_some() {
                    return Response::error(PAIRING_RUNNING);
                }
                self.wants_connection = false;
                self.error = None;
                self.reconnect_at = None;
                self.stop_link().await;
            }
            Request::Pause => self.controller.pause(),
            Request::Resume => self.controller.resume(now),
            Request::TogglePause => self.controller.toggle(now),
            Request::Dial { target } => self.controller.set_dial_target(target),
            Request::CycleDial => self.controller.cycle_dial(),
            Request::Hand { hand } => {
                if self.state != LinkState::Connected {
                    return Response::error("connect the band first");
                }
                let hand = match hand {
                    HandChoice::Left => Hand::Left,
                    HandChoice::Right => Hand::Right,
                };
                // Through the controller, so the config's hand doesn't undo the choice.
                for command in self.controller.request_hand(hand) {
                    self.apply(command);
                }
            }
            Request::Semg { on } => {
                self.semg_wanted = on;
                self.send_link(LinkCommand::RawEmg(on));
            }
            Request::Pair { address } => {
                if self.pairing.is_some() {
                    return Response::error("a pairing is already running");
                }
                return self.start_pairing(address).await;
            }
            Request::Forget => {
                if self.pairing.is_some() {
                    return Response::error(PAIRING_RUNNING);
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
                    return Response::error(format!("state: {error}"));
                }
                self.band_name = None;
                self.error = None;
                self.pairing_error = None;
                self.publish();
                return Response::message(
                    self.status.borrow().clone(),
                    format!(
                        "forgot the band. factory reset it too ({FACTORY_RESET_HINT}) before pairing it anywhere else."
                    ),
                );
            }
            Request::Subscribe | Request::SubscribeSemg => {
                return Response::error("subscriptions are handled by the control socket");
            }
        }
        self.publish();
        Response::ok(self.status.borrow().clone())
    }
}
