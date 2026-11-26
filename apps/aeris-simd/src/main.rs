//! `aeris-simd` command line.
//!
//! ```text
//! aeris-simd keygen --out <dir>                    # node, sim-controller and server keys
//! aeris-simd run --scenario s4 --seed 7            # config via flags or AERIS_* env vars
//! ```

use aeris_simd::{NatsAuth, NatsSettings, SimdConfig, StreamSettings};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

#[derive(Parser)]
#[command(name = "aeris-simd", version, about = "AERIS simulated edge daemon")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Generate Ed25519 keys (secret files 0600) and public-keys.toml.
    Keygen {
        /// Output directory (must not already contain public-keys.toml).
        #[arg(long)]
        out: PathBuf,
        /// Key ids; defaults to the world's nodes plus sim-controller and server.
        #[arg(long, value_delimiter = ',')]
        ids: Vec<String>,
    },
    /// Run a scenario.
    Run(Box<RunArgs>),
}

#[derive(clap::Args)]
struct RunArgs {
    /// Built-in scenario id (s1..s8) or path to a scenario TOML file.
    #[arg(long, env = "AERIS_SCENARIO", default_value = "s1")]
    scenario: String,
    /// World TOML (default: the frozen docs/spec/world.toml compiled in).
    #[arg(long, env = "AERIS_WORLD")]
    world: Option<PathBuf>,
    #[arg(long, env = "AERIS_SEED", default_value_t = 7)]
    seed: u64,
    #[arg(long, env = "AERIS_NATS_URL", default_value = "nats://127.0.0.1:4222")]
    nats_url: String,
    #[arg(long, env = "AERIS_NATS_USER")]
    nats_user: Option<String>,
    /// Prefer the environment variable; flags are visible in `ps`.
    #[arg(long, env = "AERIS_NATS_PASSWORD", hide_env_values = true)]
    nats_password: Option<String>,
    #[arg(long, env = "AERIS_NATS_CREDS")]
    nats_creds: Option<PathBuf>,
    /// CA bundle; setting it requires TLS.
    #[arg(long, env = "AERIS_NATS_TLS_CA")]
    nats_tls_ca: Option<PathBuf>,
    /// Directory with <node>.key and sim-controller.key (mode 0600).
    #[arg(long, env = "AERIS_KEY_DIR")]
    key_dir: PathBuf,
    /// Public key registry (must contain `server`). Default: <key-dir>/public-keys.toml.
    #[arg(long, env = "AERIS_KEY_REGISTRY")]
    registry: Option<PathBuf>,
    /// 1.0 = real time (10 Hz).
    #[arg(long, env = "AERIS_SPEED", default_value_t = 1.0)]
    speed: f64,
    #[arg(long, env = "AERIS_MAX_BUFFER", default_value_t = 20_000)]
    max_buffer: usize,
    /// Keep serving commands after the scenario ends (until SIGTERM).
    #[arg(long, env = "AERIS_HOLD")]
    hold: bool,
    /// JetStream replicas (1 locally, 3 on a clustered DC deployment).
    #[arg(long, env = "AERIS_STREAM_REPLICAS", default_value_t = 1)]
    stream_replicas: usize,
}

fn main() -> std::process::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,async_nats=warn".into()),
        )
        .with_target(false)
        .init();
    let cli = Cli::parse();
    let res = match cli.cmd {
        Cmd::Keygen { out, ids } => keygen(out, ids),
        Cmd::Run(a) => match tokio::runtime::Runtime::new() {
            Ok(rt) => rt.block_on(run(*a)),
            Err(e) => Err(format!("tokio runtime: {e}")),
        },
    };
    match res {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("{e}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn world(path: &Option<PathBuf>) -> Result<aeris_sim::WorldConfig, String> {
    match path {
        None => aeris_sim::WorldConfig::frozen().map_err(|e| e.to_string()),
        Some(p) => std::fs::read_to_string(p)
            .map_err(|e| format!("{}: {e}", p.display()))
            .and_then(|t| aeris_sim::WorldConfig::from_toml_str(&t).map_err(|e| e.to_string())),
    }
}

fn keygen(out: PathBuf, mut ids: Vec<String>) -> Result<(), String> {
    if ids.is_empty() {
        let w = world(&None)?;
        ids = w.nodes.iter().map(|n| n.id.clone()).collect();
        ids.push(aeris_crypto::keys::ids::SIM_CONTROLLER.into());
        ids.push(aeris_crypto::keys::ids::SERVER.into());
    }
    let reg = aeris_simd::keygen(&out, &ids).map_err(|e| e.to_string())?;
    println!(
        "wrote {} secret keys (0600) and public-keys.toml to {}",
        ids.len(),
        out.display()
    );
    print!("{}", reg.to_toml());
    Ok(())
}

async fn run(a: RunArgs) -> Result<(), String> {
    let w = Arc::new(world(&a.world)?);
    let scenario = if a.scenario.ends_with(".toml") {
        let t = std::fs::read_to_string(&a.scenario).map_err(|e| format!("{}: {e}", a.scenario))?;
        aeris_sim::Scenario::from_toml_str(&t, &w).map_err(|e| e.to_string())?
    } else {
        aeris_sim::Scenario::builtin(&a.scenario, &w).map_err(|e| e.to_string())?
    };
    let registry = a
        .registry
        .clone()
        .unwrap_or_else(|| a.key_dir.join("public-keys.toml"));
    let (node_keys, sim_key, server_key) =
        aeris_simd::load_keys(&w, &a.key_dir, &registry).map_err(|e| e.to_string())?;
    let auth = match (a.nats_creds, a.nats_user, a.nats_password) {
        (Some(c), _, _) => NatsAuth::CredsFile(c),
        (None, Some(user), Some(password)) => NatsAuth::UserPassword { user, password },
        (None, Some(_), None) => {
            return Err("AERIS_NATS_USER set without AERIS_NATS_PASSWORD".into());
        }
        _ => {
            tracing::warn!(
                "no NATS credentials configured (acceptable only for local development)"
            );
            NatsAuth::None
        }
    };
    let cfg = SimdConfig {
        world: w,
        scenario: Arc::new(scenario),
        seed: a.seed,
        nats: NatsSettings {
            url: a.nats_url,
            auth,
            tls_ca: a.nats_tls_ca,
            connect_attempts: 30,
        },
        stream: StreamSettings {
            replicas: a.stream_replicas,
            ..StreamSettings::default()
        },
        node_keys,
        sim_key,
        server_key,
        speed: a.speed,
        max_buffer_per_node: a.max_buffer,
        publish_timeout: Duration::from_secs(2),
        replay_after: Duration::from_secs(1),
        hold_after_end: a.hold,
        drain_timeout: Duration::from_secs(10),
    };
    let (tx, rx) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        shutdown_signal().await;
        tracing::info!("shutdown requested");
        let _ = tx.send(true);
    });
    let summary = aeris_simd::run(cfg, rx).await.map_err(|e| e.to_string())?;
    println!(
        "{}",
        serde_json::to_string_pretty(&summary).unwrap_or_default()
    );
    if summary.undelivered_at_shutdown > 0 {
        return Err(format!(
            "{} events were not delivered",
            summary.undelivered_at_shutdown
        ));
    }
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = term.recv() => {}
                }
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
