# Dharshan — Deployment readiness and Quantrix DC checklist (AERIS v0.1.0)

**Status: local deployment verified; Quantrix DC deployment NOT performed.** No DC host, DNS name, TLS certificate,
secret store or access credentials were available in this workspace, so nothing was deployed remotely and no
endpoint below is invented — every `<…>` must be filled in by whoever holds DC access.

## 1. What was verified locally (2026-11-25)

| Item | Result | How |
|---|---|---|
| Compose config renders, fails fast without secrets | ✅ | `docker compose -f deploy/compose.yaml config --quiet` (errors name the missing variable when `.env` lacks it) |
| PostgreSQL 18.7 healthy | ✅ | `pg_isready` healthcheck; `select version()` → `PostgreSQL 18.7` |
| NATS 2.15.3 + JetStream healthy | ✅ | healthcheck `GET /healthz?js-enabled-only=true` → `{"status":"ok"}`; `/varz` → `jetstream: true, auth_required: true` |
| Start-up ordering | ✅ | `nats-init` (volume ownership) → `nats` (`service_completed_successfully`); `aeris-simd` waits for `nats: service_healthy` |
| Persistent volumes | ✅ | `pgdata` → `/var/lib/postgresql` (PG 18 layout), `natsdata` → `/data/jetstream` |
| Non-root / hardening | ✅ | NATS uid 1000, read-only rootfs, `cap_drop: ALL`, `no-new-privileges`; Postgres runtime uid 999, `cap_drop: ALL` + 5 entrypoint caps |
| Restart policy | ✅ | `unless-stopped` on long-running services |
| Log rotation | ✅ | json-file, 10 MB × 5 |
| Ports | ✅ | `127.0.0.1` only: NATS 4222/8222, Postgres 55432 (5432 was taken by an unrelated local container) |
| simd → NATS (authenticated, least privilege) | ✅ | S1/S4/S7 runs: produced = published (JetStream-acked), 0 undelivered |
| Graceful shutdown | ✅ | SIGINT/SIGTERM → stop ticking, drain outbox (≤10 s), report undelivered; non-zero exit if anything was not delivered |
| simd container image | see test report §Docker | `deploy/docker/aeris-simd.Dockerfile` (pinned `rust:1.100.0`, runtime uid 10001) |
| TLS | ❌ not exercised | no certificates; template and client support prepared |
| `aeris-server` | ❌ blocked | does not exist on `main` (owner: Kesivarshini) |

## 2. Required ports

| Port | Service | Exposure in DC |
|---|---|---|
| 4222/tcp | NATS client (TLS in DC) | internal network only; edge simulators and `aeris-server` |
| 8222/tcp | NATS monitoring | internal / monitoring host only, never public |
| 6222/tcp | NATS cluster routes | only if clustering (3 nodes for JetStream R3) |
| 5432/tcp | PostgreSQL | `aeris-server` host only |
| 8080/tcp | `aeris-server` (not built) | behind TLS terminator |
| 8081/tcp | console (Trunk dev server) | dev only; production serves static files behind TLS |

## 3. Secrets (inject, never commit)

| Secret | Consumer | Injection |
|---|---|---|
| `POSTGRES_PASSWORD` | postgres, aeris-server | DC secret store → environment / Docker secret |
| `NATS_SIMD_PASSWORD`, `NATS_SERVER_PASSWORD` (or NKey `.creds`) | nats, simd, server | same; prefer NKeys (`AERIS_NATS_CREDS`) in DC |
| `node-*.key`, `sim-controller.key` | aeris-simd | read-only mount, mode 0600, owned by the container uid |
| `server.key` | aeris-server | **generated on the server host** (`aeris-simd keygen --out <dir> --ids server`), never copied off it |
| `public-keys.toml` | all | read-only mount; not secret, but integrity-critical (signed/checksummed in change control) |
| TLS key + cert + CA | nats (server), all clients (CA) | read-only mount `/etc/nats/tls` |

## 4. DC deployment procedure (to execute when access exists)

1. **Host prep** — Docker Engine ≥ 27 with Compose v2, NTP synchronised (`chronyc tracking` offset ≪ 2 s; freshness checks depend on it), firewall allowing only the ports above from the listed peers.
2. **Clone at the release tag** and verify: `git describe --tags` = `v0.1.0`.
3. **Secrets** — create `deploy/.env` from `deploy/.env.example` via the secret store (`umask 077`), set `POSTGRES_PORT`/`NATS_PORT` bindings to the internal interface, not `0.0.0.0`.
4. **TLS** — place certs in `deploy/nats/tls/`, add `include ./tls.conf` (from `tls.conf.example`) to `nats-server.conf`, mount the directory read-only, switch client URLs to `tls://<nats-host>:4222` and set `AERIS_NATS_TLS_CA`.
5. **Keys** — edge: `aeris-simd keygen --out <edge-keys> --ids node-a,node-b,node-c,sim-controller`; server host: `--ids server`; merge the two `public-keys.toml` files into one registry, review, distribute read-only.
6. **Start infra** — `docker compose -f deploy/compose.yaml up -d --wait`; confirm both services `healthy`.
7. **JetStream** — for a 3-node cluster set `AERIS_STREAM_REPLICAS=3`; the stream `AERIS_EVENTS` is created by simd on first start (or pre-create it with the same settings: file storage, max age 24 h, 1 GiB, duplicate window 120 s).
8. **Start simd** — `docker compose -f deploy/compose.yaml --profile sim up -d`; check logs for `nats connected` and `simulation started`.
9. **Smoke test** — run the NATS end-to-end suite from a host with the server credentials (`docs/audit/dharshan-techgium-readiness.md` §4) pointed at `tls://<nats-host>:4222`.
10. **Rollback** — `docker compose … down` (keeps volumes); redeploy the previous tag. Never `down -v` in the DC (destroys JetStream and Postgres data).

## 5. Logging and shutdown

* All services log to stdout (json-file driver, rotated). simd uses `tracing` (`RUST_LOG`, default `info,async_nats=warn`); passwords and secret keys are never logged (`Debug` impls redact them).
* simd exits non-zero if any event remained undelivered at shutdown, so a supervisor notices data loss.
* NATS and Postgres handle SIGTERM gracefully; compose `stop_grace_period` default (10 s) is sufficient for simd's 10 s drain.

## 6. Blockers

| Blocker | Needed from |
|---|---|
| DC host access, internal DNS names, firewall rules | Quantrix infrastructure / project owner |
| TLS certificates (or an internal CA) | Quantrix infrastructure |
| Secret store for passwords and server key | Quantrix infrastructure |
| `aeris-server` (ingest, approvals, audit store, console backend) | Kesivarshini |
| `migrations/0001_init.sql` (Postgres is healthy but has no schema) | Kesivarshini |
