//! `aeris-simd`: the simulated edge.
//!
//! Owner: DHARSHAN. Runs an [`aeris_sim::World`] at 10 Hz (scaled by
//! `speed`), signs every observation with its node's Ed25519 key and
//! publishes it to JetStream on `aeris.events.<node>`; answers signed
//! commands on `aeris.cmd.<entity>` with signed acknowledgements.
//!
//! Guarantees:
//! * **No silent loss.** An event is counted as published only after its
//!   JetStream `PublishAck`. Failed publishes are retried in order, with the
//!   same event id (JetStream de-duplicates on `Nats-Msg-Id`) and the same
//!   sequence number. Anything that could not be delivered by shutdown is
//!   reported in the [`Summary`].
//! * **Order.** One publisher, FIFO; per-node sequence numbers are assigned
//!   at creation, and a node's partition buffer is always flushed before
//!   its next live event.
//! * **S7.** Observations from a partitioned node are buffered (bounded) and
//!   replayed with `replayed = true` once its uplink returns. An event that
//!   sat in the outbox longer than `replay_after` (for example during a NATS
//!   outage) is also sent as `replayed`, so the receiver judges it by
//!   sequence number rather than rejecting it as stale.
//! * **Commands.** Only commands signed by the server key, on the subject of
//!   their target, with a fresh, increasing sequence number are applied.
//!   A re-delivered command id gets the cached acknowledgement, never a
//!   second application.

#![forbid(unsafe_code)]

use aeris_crypto::command_auth::{CommandVerifier, seal_ack};
use aeris_crypto::ingest::EventSigner;
use aeris_crypto::keys::{KeyRegistry, SigningIdentity, ids};
use aeris_crypto::replay::{ReplayConfig, SequenceCounter};
use aeris_sim::{Delivery, Observation, Scenario, SimError, World, WorldConfig};
use aeris_wire::{AckStatus, CommandAck, HlcClock, Provenance, WireEvent, WorldObservation};
use futures::StreamExt;
use rand::RngCore;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, oneshot, watch};

/// JetStream stream holding all events.
pub const STREAM_NAME: &str = "AERIS_EVENTS";

/// Daemon error.
#[derive(Debug, thiserror::Error)]
pub enum SimdError {
    /// Configuration or key problem.
    #[error("config: {0}")]
    Config(String),
    /// NATS problem that prevents running.
    #[error("nats: {0}")]
    Nats(String),
    /// Simulator problem.
    #[error("sim: {0}")]
    Sim(#[from] SimError),
}

/// How to authenticate to NATS.
#[derive(Clone, Default)]
pub enum NatsAuth {
    /// No credentials (local development only).
    #[default]
    None,
    /// User and password.
    UserPassword {
        /// User.
        user: String,
        /// Password.
        password: String,
    },
    /// `.creds` file (NKey/JWT).
    CredsFile(PathBuf),
}

impl std::fmt::Debug for NatsAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NatsAuth::None => f.write_str("None"),
            NatsAuth::UserPassword { user, .. } => write!(f, "UserPassword({user}, <redacted>)"),
            NatsAuth::CredsFile(p) => write!(f, "CredsFile({})", p.display()),
        }
    }
}

/// NATS connection settings.
#[derive(Debug, Clone)]
pub struct NatsSettings {
    /// `nats://host:4222` or `tls://host:4222`.
    pub url: String,
    /// Credentials.
    pub auth: NatsAuth,
    /// CA bundle for TLS; setting it requires TLS.
    pub tls_ca: Option<PathBuf>,
    /// Attempts for the initial connection before giving up.
    pub connect_attempts: u32,
}

/// JetStream stream settings.
#[derive(Debug, Clone)]
pub struct StreamSettings {
    /// Retention.
    pub max_age: Duration,
    /// Size cap.
    pub max_bytes: i64,
    /// JetStream de-duplication window on `Nats-Msg-Id`.
    pub duplicate_window: Duration,
    /// Replicas (1 locally, 3 in a clustered DC).
    pub replicas: usize,
}

impl Default for StreamSettings {
    fn default() -> Self {
        Self {
            max_age: Duration::from_secs(24 * 3600),
            max_bytes: 1 << 30,
            duplicate_window: Duration::from_secs(120),
            replicas: 1,
        }
    }
}

/// Everything the daemon needs.
pub struct SimdConfig {
    /// World.
    pub world: Arc<WorldConfig>,
    /// Scenario.
    pub scenario: Arc<Scenario>,
    /// Seed.
    pub seed: u64,
    /// NATS.
    pub nats: NatsSettings,
    /// Stream.
    pub stream: StreamSettings,
    /// One identity per world node, ids matching the node ids.
    pub node_keys: Vec<SigningIdentity>,
    /// Simulator controller identity (signs acks).
    pub sim_key: SigningIdentity,
    /// Server public key (verifies commands).
    pub server_key: aeris_crypto::VerifyingKey,
    /// Simulation speed: 1.0 = real time (10 Hz), 10.0 = ten times faster.
    pub speed: f64,
    /// Partition buffer capacity per node (events).
    pub max_buffer_per_node: usize,
    /// Deadline for one publish + ack.
    pub publish_timeout: Duration,
    /// Age after which an undelivered event is sent as `replayed`.
    pub replay_after: Duration,
    /// Keep answering commands after the scenario ends, until shutdown.
    pub hold_after_end: bool,
    /// Max time to drain the outbox at shutdown.
    pub drain_timeout: Duration,
}

/// Counters reported at exit.
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Summary {
    /// Ticks simulated.
    pub ticks: u64,
    /// Events created (excluding quarantined).
    pub produced: u64,
    /// Events acknowledged by JetStream.
    pub published: u64,
    /// Of which sent with `replayed = true`.
    pub replayed: u64,
    /// Publish attempts that failed and were retried.
    pub publish_retries: u64,
    /// JetStream acks flagged duplicate (already stored).
    pub duplicate_acks: u64,
    /// Observations of isolated nodes, intentionally not carried.
    pub quarantined: u64,
    /// Partition-buffer overflows (oldest dropped; logged at error level).
    pub dropped_buffer_full: u64,
    /// Largest partition buffer seen.
    pub buffered_peak: u64,
    /// Events not delivered by shutdown (outbox + partition buffers).
    pub undelivered_at_shutdown: u64,
    /// Commands applied (incl. idempotent no-ops).
    pub commands_applied: u64,
    /// Commands refused (authentication, replay, validation).
    pub commands_rejected: u64,
    /// Commands the world refused (unknown entity, invalid transition).
    pub commands_failed: u64,
    /// Re-delivered commands answered from the cache.
    pub commands_cached: u64,
}

#[derive(Default)]
struct Counters {
    produced: AtomicU64,
    published: AtomicU64,
    replayed: AtomicU64,
    retries: AtomicU64,
    dup_acks: AtomicU64,
    quarantined: AtomicU64,
    dropped: AtomicU64,
    buffered_peak: AtomicU64,
    undelivered: AtomicU64,
    applied: AtomicU64,
    rejected: AtomicU64,
    failed: AtomicU64,
    cached: AtomicU64,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Keys
// ---------------------------------------------------------------------------

/// Generates secret keys (`<id>.key`, mode 0600) and `public-keys.toml` in
/// `dir`. Refuses to overwrite any existing file.
pub fn keygen(dir: &Path, key_ids: &[String]) -> Result<KeyRegistry, SimdError> {
    std::fs::create_dir_all(dir)
        .map_err(|e| SimdError::Config(format!("{}: {e}", dir.display())))?;
    let reg_path = dir.join("public-keys.toml");
    if reg_path.exists() {
        return Err(SimdError::Config(format!(
            "{} exists; refusing to overwrite",
            reg_path.display()
        )));
    }
    let mut reg = KeyRegistry::new();
    for id in key_ids {
        let ident =
            SigningIdentity::generate(id.clone()).map_err(|e| SimdError::Config(e.to_string()))?;
        ident
            .write_secret_file(&dir.join(format!("{id}.key")))
            .map_err(|e| SimdError::Config(e.to_string()))?;
        reg.insert(id, ident.verifying_key())
            .map_err(|e| SimdError::Config(e.to_string()))?;
    }
    std::fs::write(&reg_path, reg.to_toml())
        .map_err(|e| SimdError::Config(format!("{}: {e}", reg_path.display())))?;
    Ok(reg)
}

/// Loads `<dir>/<id>.key` for every world node and the simulator controller,
/// and the server public key from `registry`.
pub fn load_keys(
    world: &WorldConfig,
    dir: &Path,
    registry: &Path,
) -> Result<
    (
        Vec<SigningIdentity>,
        SigningIdentity,
        aeris_crypto::VerifyingKey,
    ),
    SimdError,
> {
    let load = |id: &str| {
        SigningIdentity::load_secret_file(id, &dir.join(format!("{id}.key")))
            .map_err(|e| SimdError::Config(format!("key {id}: {e}")))
    };
    let nodes = world
        .nodes
        .iter()
        .map(|n| load(&n.id))
        .collect::<Result<Vec<_>, _>>()?;
    let sim = load(ids::SIM_CONTROLLER)?;
    let text = std::fs::read_to_string(registry)
        .map_err(|e| SimdError::Config(format!("{}: {e}", registry.display())))?;
    let reg = KeyRegistry::from_toml(&text).map_err(|e| SimdError::Config(e.to_string()))?;
    let server = *reg
        .get(ids::SERVER)
        .map_err(|e| SimdError::Config(format!("registry: {e}")))?;
    // The registry must agree with the private keys we hold.
    for k in nodes.iter().chain(std::iter::once(&sim)) {
        match reg.get(k.id()) {
            Ok(p) if *p == k.verifying_key() => {}
            Ok(_) => {
                return Err(SimdError::Config(format!(
                    "registry public key for {} does not match its secret key",
                    k.id()
                )));
            }
            Err(e) => return Err(SimdError::Config(format!("registry: {e}"))),
        }
    }
    Ok((nodes, sim, server))
}

// ---------------------------------------------------------------------------
// NATS
// ---------------------------------------------------------------------------

/// Connects with bounded initial retries, then unbounded reconnects.
pub async fn connect(s: &NatsSettings, name: &str) -> Result<async_nats::Client, SimdError> {
    let mut last = String::new();
    for attempt in 1..=s.connect_attempts.max(1) {
        let mut opts = async_nats::ConnectOptions::new()
            .name(name)
            .max_reconnects(None::<usize>)
            .connection_timeout(Duration::from_secs(5))
            .event_callback(|ev| async move {
                match ev {
                    async_nats::Event::Connected => tracing::info!("nats connected"),
                    async_nats::Event::Disconnected => {
                        tracing::warn!("nats disconnected; reconnecting")
                    }
                    other => tracing::warn!(event = %other, "nats event"),
                }
            });
        opts = match &s.auth {
            NatsAuth::None => opts,
            NatsAuth::UserPassword { user, password } => {
                opts.user_and_password(user.clone(), password.clone())
            }
            NatsAuth::CredsFile(p) => opts
                .credentials_file(p)
                .await
                .map_err(|e| SimdError::Config(format!("creds {}: {e}", p.display())))?,
        };
        if let Some(ca) = &s.tls_ca {
            opts = opts.add_root_certificates(ca.clone()).require_tls(true);
        }
        match opts.connect(s.url.as_str()).await {
            Ok(c) => return Ok(c),
            Err(e) => {
                last = e.to_string();
                tracing::warn!(attempt, error = %last, url = %s.url, "nats connect failed");
                tokio::time::sleep(Duration::from_millis(200 * u64::from(attempt.min(10)))).await;
            }
        }
    }
    Err(SimdError::Nats(format!(
        "could not connect to {} after {} attempts: {last}",
        s.url, s.connect_attempts
    )))
}

/// Ensures the event stream exists (creates it if allowed).
pub async fn ensure_stream(
    js: &async_nats::jetstream::Context,
    s: &StreamSettings,
) -> Result<(), SimdError> {
    use async_nats::jetstream::stream;
    if js.get_stream(STREAM_NAME).await.is_ok() {
        return Ok(());
    }
    js.get_or_create_stream(stream::Config {
        name: STREAM_NAME.to_string(),
        subjects: vec![format!("{}.>", aeris_wire::EVENTS_SUBJECT_PREFIX)],
        storage: stream::StorageType::File,
        max_age: s.max_age,
        max_bytes: s.max_bytes,
        duplicate_window: s.duplicate_window,
        num_replicas: s.replicas,
        discard: stream::DiscardPolicy::Old,
        ..Default::default()
    })
    .await
    .map(|_| ())
    .map_err(|e| SimdError::Nats(format!("stream {STREAM_NAME}: {e}")))
}

// ---------------------------------------------------------------------------
// Event pipeline
// ---------------------------------------------------------------------------

struct Pending {
    node: usize,
    event: WireEvent,
    created: Instant,
}

struct Publisher {
    js: async_nats::jetstream::Context,
    signers: Arc<Vec<EventSigner>>,
    counters: Arc<Counters>,
    timeout: Duration,
    replay_after: Duration,
}

impl Publisher {
    /// Publishes one event, retrying until acknowledged or `stop` fires.
    /// Returns `false` if it gave up because of shutdown.
    async fn publish(&self, mut p: Pending, stop: &watch::Receiver<bool>) -> bool {
        let subject = aeris_wire::event_subject(p.event.node());
        let mut backoff = Duration::from_millis(100);
        loop {
            if p.created.elapsed() > self.replay_after {
                p.event.replayed = true;
            }
            let env = match self.signers[p.node].seal(&p.event) {
                Ok(e) => e,
                Err(e) => {
                    // Only reachable with an invalid event: a bug, not a transient.
                    tracing::error!(error = %e, id = %p.event.id, "cannot seal event; dropping");
                    self.counters.undelivered.fetch_add(1, Ordering::Relaxed);
                    return true;
                }
            };
            let mut headers = async_nats::HeaderMap::new();
            headers.insert("Nats-Msg-Id", p.event.id.as_str());
            let attempt = async {
                let fut = self
                    .js
                    .publish_with_headers(subject.clone(), headers, env.into())
                    .await
                    .map_err(|e| e.to_string())?;
                fut.await.map_err(|e| e.to_string())
            };
            match tokio::time::timeout(self.timeout, attempt).await {
                Ok(Ok(ack)) => {
                    self.counters.published.fetch_add(1, Ordering::Relaxed);
                    if p.event.replayed {
                        self.counters.replayed.fetch_add(1, Ordering::Relaxed);
                    }
                    if ack.duplicate {
                        self.counters.dup_acks.fetch_add(1, Ordering::Relaxed);
                    }
                    return true;
                }
                Ok(Err(e)) => {
                    tracing::warn!(error = %e, id = %p.event.id, "publish failed; retrying")
                }
                Err(_) => tracing::warn!(id = %p.event.id, "publish ack timed out; retrying"),
            }
            self.counters.retries.fetch_add(1, Ordering::Relaxed);
            if *stop.borrow() {
                return false;
            }
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(Duration::from_secs(5));
        }
    }
}

/// Builds wire events from observations: ids, sequence numbers, HLC.
struct EventFactory {
    seq: Vec<SequenceCounter>,
    hlc: Vec<HlcClock>,
    id_rng: rand_chacha::ChaCha8Rng,
    node_index: HashMap<String, usize>,
}

impl EventFactory {
    fn new(world: &WorldConfig, seed: u64) -> Self {
        Self {
            seq: vec![SequenceCounter::new(); world.nodes.len()],
            hlc: vec![HlcClock::new(2_000); world.nodes.len()],
            id_rng: aeris_sim::rng::stream(seed, "simd-event-ids", 0),
            node_index: world
                .nodes
                .iter()
                .enumerate()
                .map(|(i, n)| (n.id.clone(), i))
                .collect(),
        }
    }

    fn make(&mut self, o: &Observation, t_ms: i64) -> Result<(usize, WireEvent), SimdError> {
        let node = *self
            .node_index
            .get(&o.node)
            .ok_or_else(|| SimdError::Config(format!("unknown node {}", o.node)))?;
        let seq = self.seq[node]
            .advance()
            .map_err(|e| SimdError::Config(format!("{}: {e}", o.node)))?;
        let hlc = self.hlc[node]
            .send(t_ms)
            .map_err(|e| SimdError::Config(format!("{}: {e}", o.node)))?;
        let mut rnd = [0u8; 10];
        self.id_rng.fill_bytes(&mut rnd);
        let id = uuid::Builder::from_unix_timestamp_millis(t_ms.max(0) as u64, &rnd)
            .into_uuid()
            .hyphenated()
            .to_string();
        Ok((
            node,
            WireEvent {
                id,
                source: o.node.clone(),
                seq,
                hlc,
                t_unix_ms: t_ms,
                replayed: false,
                confidence: o.confidence,
                provenance: Provenance {
                    producer: o.producer.clone(),
                    producer_version: aeris_sim::world::PRODUCER_VERSION.to_string(),
                },
                payload: o.payload.clone(),
            },
        ))
    }
}

struct CommandJob {
    op: aeris_wire::CommandOp,
    reply: oneshot::Sender<(Result<aeris_sim::ApplyOutcome, SimError>, WorldObservation)>,
}

// ---------------------------------------------------------------------------
// Daemon
// ---------------------------------------------------------------------------

/// Runs the daemon until the scenario ends (or, with `hold_after_end`, until
/// `shutdown` becomes `true`). Always drains before returning.
pub async fn run(
    cfg: SimdConfig,
    mut shutdown: watch::Receiver<bool>,
) -> Result<Summary, SimdError> {
    if cfg.node_keys.len() != cfg.world.nodes.len()
        || cfg
            .node_keys
            .iter()
            .zip(&cfg.world.nodes)
            .any(|(k, n)| k.id() != n.id)
    {
        return Err(SimdError::Config(
            "node keys must match world nodes in order".into(),
        ));
    }
    if !(cfg.speed.is_finite() && cfg.speed > 0.0) {
        return Err(SimdError::Config("speed must be > 0".into()));
    }
    let mut world = World::new(Arc::clone(&cfg.world), Arc::clone(&cfg.scenario), cfg.seed)?;
    let client = connect(&cfg.nats, "aeris-simd").await?;
    let js = async_nats::jetstream::new(client.clone());
    ensure_stream(&js, &cfg.stream).await?;

    let counters = Arc::new(Counters::default());
    let signers: Arc<Vec<EventSigner>> =
        Arc::new(cfg.node_keys.into_iter().map(EventSigner::new).collect());
    let (stop_tx, stop_rx) = watch::channel(false);

    // Publisher: single FIFO consumer.
    let (ev_tx, mut ev_rx) = mpsc::channel::<Pending>(65_536);
    let publisher = Publisher {
        js,
        signers: Arc::clone(&signers),
        counters: Arc::clone(&counters),
        timeout: cfg.publish_timeout,
        replay_after: cfg.replay_after,
    };
    let pub_stop = stop_rx.clone();
    let pub_counters = Arc::clone(&counters);
    let publisher_task = tokio::spawn(async move {
        while let Some(p) = ev_rx.recv().await {
            if !publisher.publish(p, &pub_stop).await {
                // Shutdown while undeliverable: count this one and the rest.
                let mut n = 1u64;
                while ev_rx.try_recv().is_ok() {
                    n += 1;
                }
                pub_counters.undelivered.fetch_add(n, Ordering::Relaxed);
                tracing::error!(undelivered = n, "shutdown with undelivered events");
                break;
            }
        }
    });

    // Command handler.
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<CommandJob>(64);
    let sub = client
        .subscribe(format!("{}.>", aeris_wire::CMD_SUBJECT_PREFIX))
        .await
        .map_err(|e| SimdError::Nats(format!("subscribe: {e}")))?;
    let cmd_client = client.clone();
    let cmd_counters = Arc::clone(&counters);
    let sim_key = cfg.sim_key;
    let mut verifier = CommandVerifier::new(ids::SERVER, cfg.server_key, ReplayConfig::default());
    let mut cmd_stop = stop_rx.clone();
    let command_task = tokio::spawn(async move {
        let mut sub = sub;
        let mut cache: HashMap<String, Vec<u8>> = HashMap::new();
        let mut cache_order: VecDeque<String> = VecDeque::new();
        loop {
            let msg = tokio::select! {
                m = sub.next() => match m { Some(m) => m, None => break },
                _ = cmd_stop.changed() => break,
            };
            let Some(reply) = msg.reply.clone() else {
                tracing::warn!(subject = %msg.subject, "command without reply subject ignored");
                cmd_counters.rejected.fetch_add(1, Ordering::Relaxed);
                continue;
            };
            let now = now_ms();
            let rejected = |id: &str, reason: String| CommandAck {
                command_id: if id.is_empty() {
                    "unknown".into()
                } else {
                    id.to_string()
                },
                status: AckStatus::Rejected,
                reason,
                observation: None,
                t_unix_ms: now,
            };
            let ack = match verifier.authenticate(msg.subject.as_str(), &msg.payload) {
                Err(e) => {
                    tracing::warn!(subject = %msg.subject, error = %e, "command rejected");
                    cmd_counters.rejected.fetch_add(1, Ordering::Relaxed);
                    // Echo the *claimed* id (if the envelope decodes) so the
                    // sender can correlate the rejection. Safe: the reply goes
                    // only to the sender's inbox, the command had no effect,
                    // and REJECTED is never treated as success.
                    let claimed = aeris_wire::decode_command_envelope(&msg.payload)
                        .ok()
                        .and_then(|raw| aeris_wire::decode_command(&raw.body).ok())
                        .map(|c| c.id)
                        .unwrap_or_default();
                    rejected(&claimed, e.to_string())
                }
                Ok(cmd) => {
                    if let Some(bytes) = cache.get(&cmd.id) {
                        cmd_counters.cached.fetch_add(1, Ordering::Relaxed);
                        let _ = cmd_client.publish(reply, bytes.clone().into()).await;
                        continue;
                    }
                    match verifier.admit_seq(&cmd, now) {
                        Err(e) => {
                            tracing::warn!(id = %cmd.id, error = %e, "command replay rejected");
                            cmd_counters.rejected.fetch_add(1, Ordering::Relaxed);
                            rejected(&cmd.id, e.to_string())
                        }
                        Ok(()) => {
                            let (tx, rx) = oneshot::channel();
                            let sent = cmd_tx
                                .send(CommandJob {
                                    op: cmd.op.clone(),
                                    reply: tx,
                                })
                                .await;
                            let res = match sent {
                                Ok(()) => tokio::time::timeout(Duration::from_secs(5), rx)
                                    .await
                                    .ok()
                                    .and_then(Result::ok),
                                Err(_) => None,
                            };
                            match res {
                                Some((r, obs)) => {
                                    if r.is_ok() {
                                        cmd_counters.applied.fetch_add(1, Ordering::Relaxed);
                                    } else {
                                        cmd_counters.failed.fetch_add(1, Ordering::Relaxed);
                                    }
                                    tracing::info!(id = %cmd.id, decision = %cmd.decision_id, op = ?cmd.op, result = ?r, "command processed");
                                    let mut a = aeris_exec::ack_for(&cmd.id, r, obs);
                                    a.t_unix_ms = now_ms();
                                    a
                                }
                                None => {
                                    cmd_counters.failed.fetch_add(1, Ordering::Relaxed);
                                    CommandAck {
                                        command_id: cmd.id.clone(),
                                        status: AckStatus::Failed,
                                        reason: "simulator is not running".into(),
                                        observation: None,
                                        t_unix_ms: now_ms(),
                                    }
                                }
                            }
                        }
                    }
                }
            };
            match seal_ack(&sim_key, &ack) {
                Ok(bytes) => {
                    if ack.status != AckStatus::Rejected {
                        cache.insert(ack.command_id.clone(), bytes.clone());
                        cache_order.push_back(ack.command_id.clone());
                        while cache_order.len() > 1_024 {
                            if let Some(old) = cache_order.pop_front() {
                                cache.remove(&old);
                            }
                        }
                    }
                    if let Err(e) = cmd_client.publish(reply, bytes.into()).await {
                        tracing::warn!(error = %e, "ack publish failed");
                    }
                }
                Err(e) => tracing::error!(error = %e, "cannot seal ack"),
            }
        }
    });

    // Tick loop: owns the world.
    let tick = Duration::from_secs_f64(f64::from(world.tick_ms() as u32) / 1_000.0 / cfg.speed);
    let mut interval = tokio::time::interval(tick);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let duration = cfg.scenario.duration_ticks(cfg.world.world.tick_hz);
    let mut factory = EventFactory::new(&cfg.world, cfg.seed);
    let mut buffers: Vec<VecDeque<Pending>> = (0..cfg.world.nodes.len())
        .map(|_| VecDeque::new())
        .collect();
    let mut ended = false;
    tracing::info!(scenario = %cfg.scenario.scenario.id, seed = cfg.seed, speed = cfg.speed, "simulation started");
    let mut shutdown_live = true;
    loop {
        tokio::select! {
            _ = interval.tick() => {}
            r = shutdown.changed(), if shutdown_live => {
                match r {
                    Ok(()) if *shutdown.borrow() => break,
                    Ok(()) => {}
                    // Sender gone: no shutdown signal can arrive any more.
                    Err(_) => shutdown_live = false,
                }
                continue;
            }
        }
        while let Ok(job) = cmd_rx.try_recv() {
            let r = world.apply_command(&job.op);
            let _ = job.reply.send((r, world.observe()));
        }
        if ended {
            continue;
        }
        if world.tick() >= duration {
            ended = true;
            tracing::info!(ticks = world.tick(), "scenario complete");
            if !cfg.hold_after_end {
                break;
            }
            continue;
        }
        let t = now_ms();
        for o in world.step() {
            match o.delivery {
                Delivery::Quarantined => {
                    counters.quarantined.fetch_add(1, Ordering::Relaxed);
                }
                Delivery::Partitioned => {
                    let (node, ev) = factory.make(&o, t)?;
                    counters.produced.fetch_add(1, Ordering::Relaxed);
                    let b = &mut buffers[node];
                    if b.len() >= cfg.max_buffer_per_node {
                        b.pop_front();
                        counters.dropped.fetch_add(1, Ordering::Relaxed);
                        tracing::error!(node = %o.node, cap = cfg.max_buffer_per_node, "partition buffer full; oldest event dropped");
                    }
                    b.push_back(Pending {
                        node,
                        event: WireEvent {
                            replayed: true,
                            ..ev
                        },
                        created: Instant::now(),
                    });
                    counters
                        .buffered_peak
                        .fetch_max(b.len() as u64, Ordering::Relaxed);
                }
                Delivery::Uplink => {
                    let (node, ev) = factory.make(&o, t)?;
                    counters.produced.fetch_add(1, Ordering::Relaxed);
                    if !buffers[node].is_empty() {
                        tracing::info!(node = %o.node, n = buffers[node].len(), "uplink restored; replaying buffer");
                    }
                    while let Some(p) = buffers[node].pop_front() {
                        if ev_tx.send(p).await.is_err() {
                            return Err(SimdError::Nats("publisher stopped".into()));
                        }
                    }
                    if ev_tx
                        .send(Pending {
                            node,
                            event: ev,
                            created: Instant::now(),
                        })
                        .await
                        .is_err()
                    {
                        return Err(SimdError::Nats("publisher stopped".into()));
                    }
                }
            }
        }
    }

    // Shutdown: flush what can be flushed, count what cannot.
    let stranded: u64 = buffers.iter().map(|b| b.len() as u64).sum();
    if stranded > 0 {
        tracing::error!(
            events = stranded,
            "partition buffers not replayable at shutdown"
        );
        counters.undelivered.fetch_add(stranded, Ordering::Relaxed);
    }
    drop(ev_tx);
    let ticks = world.tick();
    let mut publisher_task = publisher_task;
    if tokio::time::timeout(cfg.drain_timeout, &mut publisher_task)
        .await
        .is_err()
    {
        let _ = stop_tx.send(true);
        let _ = publisher_task.await;
    }
    let _ = stop_tx.send(true);
    let _ = command_task.await;
    let _ = client.flush().await;
    let c = &counters;
    let s = Summary {
        ticks,
        produced: c.produced.load(Ordering::Relaxed),
        published: c.published.load(Ordering::Relaxed),
        replayed: c.replayed.load(Ordering::Relaxed),
        publish_retries: c.retries.load(Ordering::Relaxed),
        duplicate_acks: c.dup_acks.load(Ordering::Relaxed),
        quarantined: c.quarantined.load(Ordering::Relaxed),
        dropped_buffer_full: c.dropped.load(Ordering::Relaxed),
        buffered_peak: c.buffered_peak.load(Ordering::Relaxed),
        undelivered_at_shutdown: c.undelivered.load(Ordering::Relaxed),
        commands_applied: c.applied.load(Ordering::Relaxed),
        commands_rejected: c.rejected.load(Ordering::Relaxed),
        commands_failed: c.failed.load(Ordering::Relaxed),
        commands_cached: c.cached.load(Ordering::Relaxed),
    };
    tracing::info!(summary = ?s, "aeris-simd stopped");
    Ok(s)
}
