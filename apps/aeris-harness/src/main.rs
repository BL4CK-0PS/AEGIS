//! `aeris-harness` command line.
//!
//! ```text
//! aeris-harness run --scenario s4 --seed 7 --approve --narrate
//! aeris-harness sweep --scenarios s1,s2,s3,s4,s5,s6,s7,s8 --seeds 100 --approve --out-dir docs/testing/results
//! aeris-harness verify-audit --file audit.jsonl --registry deploy/keys/public-keys.toml
//! aeris-harness live --key-dir deploy/keys                     # with aeris-simd running; type 'approve'
//! AERIS_DATABASE_URL=postgres://… aeris-harness run --scenario s4 --approve --store-pg
//! AERIS_DATABASE_URL=postgres://… aeris-harness verify-audit --pg --registry deploy/keys/public-keys.toml
//! ```
//! `--approve` enables the explicit test commander. Without it, HUMAN_REVIEW
//! proposals are never executed.

use aeris_harness::closed_loop::{ApprovalPolicy, RunConfig, RunResult, Strategy, run};
use aeris_harness::sweep::{aggregate, markdown, runs_csv};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Parser)]
#[command(name = "aeris-harness", version, about = "AERIS closed-loop harness")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// One scenario and seed.
    Run {
        #[arg(long, default_value = "s4")]
        scenario: String,
        #[arg(long, default_value_t = 7)]
        seed: u64,
        #[arg(long, value_enum, default_value = "aeris")]
        strategy: StrategyArg,
        /// Enable the explicit test commander for HUMAN_REVIEW proposals.
        #[arg(long)]
        approve: bool,
        /// Simulated human reaction time, ticks.
        #[arg(long, default_value_t = 0)]
        approval_delay_ticks: u64,
        #[arg(long)]
        narrate: bool,
        /// Write the audit chain as JSONL.
        #[arg(long)]
        audit_out: Option<PathBuf>,
        /// Server secret key (0600) used to sign audit records.
        #[arg(long, env = "AERIS_SERVER_KEY")]
        server_key: Option<PathBuf>,
        /// Apply migrations/0001_init.sql and write the audit chain to
        /// PostgreSQL (`AERIS_DATABASE_URL`; `audit_log` must be empty).
        #[arg(long)]
        store_pg: bool,
    },
    /// Scenarios × seeds × strategies; writes CSV and Markdown.
    Sweep {
        #[arg(long, value_delimiter = ',', default_value = "s1,s2,s3,s4,s5,s6,s7,s8")]
        scenarios: Vec<String>,
        #[arg(long, default_value_t = 20)]
        seeds: u64,
        #[arg(long, default_value_t = 1)]
        first_seed: u64,
        #[arg(long, value_delimiter = ',', default_value = "aeris,alert-only,naive")]
        strategies: Vec<StrategyArg>,
        #[arg(long)]
        approve: bool,
        #[arg(long, default_value_t = 0)]
        approval_delay_ticks: u64,
        #[arg(long)]
        out_dir: PathBuf,
        /// Parallel runs (results are identical; latency is measured under load).
        #[arg(long, default_value_t = 1)]
        jobs: usize,
    },
    /// Closed loop against the running stack: consume aeris-simd's signed
    /// events from JetStream, run aeris-core, ask the operator to approve,
    /// execute over NATS, verify, audit.
    Live {
        #[arg(long, env = "AERIS_NATS_URL", default_value = "nats://127.0.0.1:4222")]
        nats_url: String,
        /// Control-plane NATS user (NATS_SERVER_USER in deploy/.env).
        #[arg(long, env = "AERIS_NATS_USER")]
        nats_user: Option<String>,
        /// Prefer the environment variable; flags are visible in `ps`.
        #[arg(long, env = "AERIS_NATS_PASSWORD", hide_env_values = true)]
        nats_password: Option<String>,
        /// Directory with public-keys.toml and server.key.
        #[arg(long, env = "AERIS_KEY_DIR")]
        key_dir: PathBuf,
        #[arg(long, default_value_t = 75.0)]
        duration_s: f64,
        #[arg(long, default_value_t = 7)]
        seed: u64,
        /// Operator id recorded with a typed approval.
        #[arg(long, default_value = "commander-1")]
        operator: String,
        /// Observe only: never execute anything that needs a human.
        #[arg(long)]
        no_approval: bool,
        #[arg(long)]
        audit_out: Option<PathBuf>,
        /// Also write the audit chain to PostgreSQL (AERIS_DATABASE_URL).
        #[arg(long)]
        store_pg: bool,
    },
    /// Verify a JSONL audit export.
    VerifyAudit {
        /// JSONL export to verify.
        #[arg(long, required_unless_present = "pg", conflicts_with = "pg")]
        file: Option<PathBuf>,
        /// Verify the `audit_log` table in PostgreSQL (`AERIS_DATABASE_URL`).
        #[arg(long)]
        pg: bool,
        #[arg(long)]
        registry: PathBuf,
    },
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum StrategyArg {
    Aeris,
    AlertOnly,
    Naive,
}

impl From<StrategyArg> for Strategy {
    fn from(s: StrategyArg) -> Self {
        match s {
            StrategyArg::Aeris => Strategy::Aeris,
            StrategyArg::AlertOnly => Strategy::AlertOnly,
            StrategyArg::Naive => Strategy::Naive,
        }
    }
}

fn main() -> std::process::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "error".into()),
        )
        .init();
    match real_main() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn real_main() -> Result<std::process::ExitCode, String> {
    let world = Arc::new(aeris_sim::WorldConfig::frozen().map_err(|e| e.to_string())?);
    match Cli::parse().cmd {
        Cmd::Run {
            scenario,
            seed,
            strategy,
            approve,
            approval_delay_ticks,
            narrate,
            audit_out,
            server_key,
            store_pg,
        } => {
            let sc = Arc::new(
                aeris_sim::Scenario::builtin(&scenario, &world).map_err(|e| e.to_string())?,
            );
            let cfg = RunConfig {
                world,
                scenario: sc,
                seed,
                strategy: strategy.into(),
                approval: ApprovalPolicy {
                    test_commander_approves: approve,
                    delay_ticks: approval_delay_ticks,
                },
                narrate,
                server_key_file: server_key,
            };
            let (r, entries) = run(&cfg)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&r).map_err(|e| e.to_string())?
            );
            if let Some(p) = audit_out {
                let text = aeris_crypto::audit::to_jsonl(&entries).map_err(|e| e.to_string())?;
                std::fs::write(&p, text).map_err(|e| format!("{}: {e}", p.display()))?;
            }
            if store_pg {
                let mut c = aeris_harness::pg::connect(&database_url()?)?;
                aeris_harness::pg::migrate(&mut c)?;
                let n = aeris_harness::pg::store_audit(&mut c, &entries)?;
                eprintln!("stored {n} audit records in PostgreSQL audit_log");
            }
            Ok(std::process::ExitCode::SUCCESS)
        }
        Cmd::Sweep {
            scenarios,
            seeds,
            first_seed,
            strategies,
            approve,
            approval_delay_ticks,
            out_dir,
            jobs,
        } => {
            std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
            let started = std::time::Instant::now();
            let mut cfgs: Vec<RunConfig> = Vec::new();
            for sc_id in &scenarios {
                let sc = Arc::new(
                    aeris_sim::Scenario::builtin(sc_id, &world).map_err(|e| e.to_string())?,
                );
                for st in &strategies {
                    for seed in first_seed..first_seed + seeds {
                        cfgs.push(RunConfig {
                            world: Arc::clone(&world),
                            scenario: Arc::clone(&sc),
                            seed,
                            strategy: (*st).into(),
                            approval: ApprovalPolicy {
                                test_commander_approves: approve,
                                delay_ticks: approval_delay_ticks,
                            },
                            narrate: false,
                            server_key_file: None,
                        });
                    }
                }
            }
            let next = std::sync::atomic::AtomicUsize::new(0);
            let done = std::sync::atomic::AtomicUsize::new(0);
            let slots: Vec<std::sync::Mutex<Option<Result<RunResult, String>>>> =
                cfgs.iter().map(|_| std::sync::Mutex::new(None)).collect();
            std::thread::scope(|scope| {
                for _ in 0..jobs.max(1) {
                    scope.spawn(|| {
                        loop {
                            let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            let Some(c) = cfgs.get(i) else { break };
                            let r = run(c).map(|(r, _)| r);
                            if let Ok(mut slot) = slots[i].lock() {
                                *slot = Some(r);
                            }
                            let d = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                            if d.is_multiple_of(50) || d == cfgs.len() {
                                eprintln!(
                                    "{d}/{} runs ({:.0}s)",
                                    cfgs.len(),
                                    started.elapsed().as_secs_f64()
                                );
                            }
                        }
                    });
                }
            });
            let mut results: Vec<RunResult> = Vec::with_capacity(cfgs.len());
            for s in slots {
                let r = s.into_inner().map_err(|_| "worker panicked".to_string())?;
                results.push(r.ok_or("run missing")??);
            }
            let aggs = aggregate(&results);
            let header = format!(
                "## aeris-harness sweep\n\nScenarios {}, seeds {}..={}, strategies {}, test approval {} (delay {} ticks). Generated by `aeris-harness sweep`; all values measured.",
                scenarios.join(","),
                first_seed,
                first_seed + seeds - 1,
                strategies
                    .iter()
                    .map(|s| Strategy::from(*s).to_string())
                    .collect::<Vec<_>>()
                    .join(","),
                if approve { "ON" } else { "OFF" },
                approval_delay_ticks
            );
            let md = markdown(&aggs, &header);
            std::fs::write(out_dir.join("runs.csv"), runs_csv(&results))
                .map_err(|e| e.to_string())?;
            std::fs::write(out_dir.join("summary.md"), &md).map_err(|e| e.to_string())?;
            std::fs::write(
                out_dir.join("summary.json"),
                serde_json::to_string_pretty(&aggs).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            println!("{md}");
            Ok(std::process::ExitCode::SUCCESS)
        }
        Cmd::Live {
            nats_url,
            nats_user,
            nats_password,
            key_dir,
            duration_s,
            seed,
            operator,
            no_approval,
            audit_out,
            store_pg,
        } => {
            let auth = match (nats_user, nats_password) {
                (Some(user), Some(password)) => {
                    aeris_simd::NatsAuth::UserPassword { user, password }
                }
                (None, None) => aeris_simd::NatsAuth::None,
                _ => return Err("set both AERIS_NATS_USER and AERIS_NATS_PASSWORD".into()),
            };
            let cfg = aeris_harness::live::LiveConfig {
                nats_url,
                auth,
                key_dir,
                duration_s,
                seed,
                approver: if no_approval {
                    aeris_harness::live::Approver::Never
                } else {
                    aeris_harness::live::Approver::Interactive { operator }
                },
                audit_out,
            };
            let rt = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
            let entries = rt.block_on(aeris_harness::live::run_live(cfg))?;
            if store_pg {
                let mut c = aeris_harness::pg::connect(&database_url()?)?;
                aeris_harness::pg::migrate(&mut c)?;
                let n = aeris_harness::pg::store_audit(&mut c, &entries)?;
                eprintln!("stored {n} audit records in PostgreSQL audit_log");
            }
            Ok(std::process::ExitCode::SUCCESS)
        }
        Cmd::VerifyAudit { file, pg, registry } => {
            let r = match (file, pg) {
                (Some(f), false) => aeris_harness::verify_audit_file(&f, &registry, None)?,
                (None, true) => {
                    let mut c = aeris_harness::pg::connect(&database_url()?)?;
                    let entries = aeris_harness::pg::load_audit(&mut c)?;
                    aeris_harness::verify_audit_entries(&entries, &registry, None)?
                }
                _ => return Err("give exactly one of --file or --pg".into()),
            };
            println!(
                "{}",
                serde_json::to_string_pretty(&r).map_err(|e| e.to_string())?
            );
            Ok(if r.ok {
                std::process::ExitCode::SUCCESS
            } else {
                std::process::ExitCode::from(2)
            })
        }
    }
}

fn database_url() -> Result<String, String> {
    std::env::var("AERIS_DATABASE_URL")
        .map_err(|_| "set AERIS_DATABASE_URL (postgres://user:password@host:port/db)".to_string())
}
