# 17 — Deployment and Docker Architecture

## 1. Purpose

This document defines how AERIS is packaged, deployed, executed, and scaled using containers.

The deployment architecture must support:

* Local development with rapid hot-reloading
* Synthetic benchmark execution across six machine environments
* Live hackathon demonstration and judging
* Fully reproducible, bit-for-bit deployment
* Isolated asynchronous processing workers
* Local AI inference via Ollama or mock providers
* Resilient PostgreSQL persistence with migrations
* High-performance frontend investigation workbench
* Future cloud and on-premise production scaling

The primary deployment principle is:

> **Every component should be independently replaceable without changing the AERIS evidence model.**

---

# 2. Deployment Overview

```mermaid
flowchart TD
    Browser["User / Investigator Web Browser"] --> Frontend["Frontend Container (Nginx / Vite UI :3000)"]
    Frontend -->|HTTP REST & SSE :8080| API["AERIS API Container (Rust Axum :8080)"]
    
    subgraph CoreBackend ["AERIS Core Backend Infrastructure"]
        API --> DB[("PostgreSQL 17 Container :5432")]
        API --> Workers["Asynchronous Workers (Rust Pipeline Worker)"]
        Workers --> DB
        Workers --> AI["AI Provider / Ollama Container :11434"]
        Workers --> Storage[("Local Object Storage / Docker Volumes")]
    end
    
    subgraph SyntheticMachines ["Isolated Machine Simulation Services"]
        M1["machine-m001 (Press-A)"]
        M2["machine-m002 (Pump-B)"]
        M3["machine-m003 (Motor-C)"]
        M4["machine-m004 (Compressor-D)"]
        M5["machine-m005 (Conveyor-E)"]
        M6["machine-m006 (Robot-F)"]
    end
    
    M1 -->|Generates Evidence| Storage
    M2 -->|Generates Evidence| Storage
    M3 -->|Generates Evidence| Storage
    M4 -->|Generates Evidence| Storage
    M5 -->|Generates Evidence| Storage
    M6 -->|Generates Evidence| Storage
```

```text
                        Browser
                           │
                           ▼
                    ┌────────────┐
                    │  Frontend  │
                    └─────┬──────┘
                          │
                          ▼
                    ┌────────────┐
                    │  API       │
                    │  Axum      │
                    └─────┬──────┘
                          │
             ┌────────────┼────────────┐
             ▼            ▼            ▼
        PostgreSQL    Processing    AI Provider
                       Workers       / Ollama
             │            │            │
             └────────────┼────────────┘
                          │
                          ▼
                   Object Storage
```

The synthetic machine environment operates as an isolated generator cluster:

```text
M-001  (Press-A)
M-002  (Pump-B)
M-003  (Motor-C)
M-004  (Compressor-D)
M-005  (Conveyor-E)
M-006  (Robot-F)
```

Each machine runs in its own isolated container and outputs to a dedicated volume mount.

---

# 3. Deployment Environments

AERIS supports four distinct deployment profiles:

```text
Development ──► Benchmark ──► Demo ──► Production
```

* **Development:** Fast rebuilds, SQLx offline mode, mock or local Ollama, bind mounts for local files.
* **Benchmark:** Headless execution, fixed random seed, isolated ground-truth evaluator, JSON artifact output.
* **Demo:** Complete end-to-end containerized UI, pre-seeded synthetic dataset, live SSE progress streaming.
* **Production:** Multi-worker distribution, reverse proxy TLS termination, separate GPU nodes, managed database.

---

# 4. Development Environment

Development prioritizes instantaneous feedback:

```bash
# Start backend infrastructure only
docker compose up -d postgres ollama

# Run API and worker locally in watch mode
cargo watch -x "run --bin aeris-api"
cargo watch -x "run --bin aeris-worker"

# Run frontend dev server
npm run dev
```

---

# 5. Benchmark Environment

Benchmark mode enforces deterministic execution:

* Pinned random seed (`AERIS_DATASET_SEED=20260930`)
* Fixed configuration snapshot (`aeris-benchmark.toml`)
* Isolated ground-truth directory mounted read-only
* Automated execution of all six machine cases
* Generation of `evaluation.json` and `summary.md`

Executed via:

```bash
docker compose --profile benchmark up --abort-on-container-exit
```

---

# 6. Demonstration Environment

The Techgium evaluation and demo environment boots the entire stack from a cold state:

```bash
docker compose up -d
./scripts/seed-demo.sh
```

The seed script initializes the database, generates synthetic evidence across all six machines, creates the demo cases, runs the pipeline, and prepares the workbench UI.

---

# 7. Production Architecture

High-throughput enterprise topology:

```mermaid
flowchart TD
    Client["Client / Browser"] --> RP["Reverse Proxy / Ingress (Traefik / Nginx)"]
    RP -->|TLS / Frontend| Static["Static Web Host / CDN"]
    RP -->|API / WebSocket / SSE| APICluster["Axum API Gateway Cluster"]
    
    APICluster --> Queue["Distributed Job Queue (NATS / Redis Streams)"]
    APICluster --> PGMaster[("PostgreSQL 17 Primary")]
    APICluster --> PGReplica[("PostgreSQL Read Replicas")]
    
    Queue --> CPUWorkers["CPU Worker Pool (Normalizer, ER, Conflict)"]
    Queue --> GPUWorkers["GPU Worker Pool (Vision OCR, Local LLM)"]
    
    CPUWorkers --> PGMaster
    GPUWorkers --> PGMaster
    
    CPUWorkers --> S3[("MinIO / AWS S3 Object Storage")]
    GPUWorkers --> S3
```

```text
                  Reverse Proxy
                       │
              ┌────────┴────────┐
              ▼                 ▼
           Frontend            API
                                  │
                    ┌─────────────┼─────────────┐
                    ▼             ▼             ▼
                Queue         PostgreSQL    Object Store
                    │
              ┌─────┴─────┐
              ▼           ▼
          CPU Worker   GPU Worker
```

The initial hackathon delivery remains lean and self-contained on a single host.

---

# 8. Docker Compose Architecture

Root repository layout:

```text
aeris/
├── docker-compose.yml
├── docker-compose.override.yml
├── .env.example
├── docker/
│   ├── api/
│   │   └── Dockerfile
│   ├── worker/
│   │   └── Dockerfile
│   ├── frontend/
│   │   ├── Dockerfile
│   │   └── nginx.conf
│   └── machines/
│       ├── Dockerfile
│       └── simulator.py
│
├── crates/
├── apps/
├── datasets/
│   ├── generated/
│   └── ground-truth/
├── benchmark/
└── scripts/
    ├── seed-demo.sh
    └── reset-demo.sh
```

---

# 9. Core Services

Defined in `docker-compose.yml`:

```text
postgres      (Database metadata, reasoning state, event stream)
api           (Rust Axum HTTP & SSE API server)
worker        (Asynchronous pipeline orchestration & reasoning)
frontend      (Nginx web server hosting compiled UI)
ollama        (Local LLM and vision model inference)
```

Optional services enabled via profiles:

```text
minio         (S3-compatible object storage)
redis         (Distributed job queue)
evaluator     (Independent benchmark scoring harness)
```

---

# 10. Synthetic Machine Services

Six independent simulation containers for machines M-001 through M-006:

```text
machine-m001  (Press-A: Normal baseline operation)
machine-m002  (Pump-B: 21.7°C temperature conflict)
machine-m003  (Motor-C: Unit mismatch 158°F vs. 70°C)
machine-m004  (Compressor-D: Telemetry dropout gap)
machine-m005  (Conveyor-E: Alias ambiguity CV-E vs. CV-E2)
machine-m006  (Robot-F: Multimodal visual thermal disagreement)
```

---

# 11. Machine Isolation

Each machine container mounts only its assigned isolated directory:

```text
datasets/generated/
├── M-001/
├── M-002/
├── M-003/
├── M-004/
├── M-005/
└── M-006/
```

```text
Container: machine-m001 ──► /output/M-001
Container: machine-m002 ──► /output/M-002
```

This ensures complete file and memory isolation between machine simulations.

---

# 12. Synthetic Machine Dockerfile

`docker/machines/Dockerfile`:

```dockerfile
FROM python:3.12-slim

WORKDIR /app

COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt

COPY simulator.py .

ENV PYTHONUNBUFFERED=1

ENTRYPOINT ["python", "simulator.py"]
```

---

# 13. Machine Compose Definition

```yaml
services:
  machine-m001:
    build:
      context: ./docker/machines
    container_name: aeris-machine-m001
    environment:
      MACHINE_ID: M-001
      SEED: 20260930
    volumes:
      - ./datasets/generated/M-001:/output
    profiles: ["benchmark", "demo"]

  machine-m002:
    build:
      context: ./docker/machines
    container_name: aeris-machine-m002
    environment:
      MACHINE_ID: M-002
      SEED: 20260930
    volumes:
      - ./datasets/generated/M-002:/output
    profiles: ["benchmark", "demo"]
```

Machines M-003 through M-006 follow this identical pattern.

---

# 14. PostgreSQL Container

```yaml
postgres:
  image: postgres:17-alpine
  container_name: aeris-postgres
  restart: unless-stopped
  environment:
    POSTGRES_DB: ${POSTGRES_DB:-aeris}
    POSTGRES_USER: ${POSTGRES_USER:-aeris}
    POSTGRES_PASSWORD: ${POSTGRES_PASSWORD}
  volumes:
    - postgres_data:/var/lib/postgresql/data
  ports:
    - "127.0.0.1:5432:5432"
  networks:
    - aeris-net
  healthcheck:
    test: ["CMD-SHELL", "pg_isready -U ${POSTGRES_USER:-aeris} -d ${POSTGRES_DB:-aeris}"]
    interval: 3s
    timeout: 3s
    retries: 10
```

---

# 15. PostgreSQL Health Check

The health check confirms that PostgreSQL is accepting connections before dependent services start:

```yaml
healthcheck:
  test: ["CMD-SHELL", "pg_isready -U aeris -d aeris"]
  interval: 3s
  timeout: 3s
  retries: 10
```

---

# 16. API Container

```yaml
api:
  build:
    context: .
    dockerfile: docker/api/Dockerfile
  container_name: aeris-api
  restart: unless-stopped
  depends_on:
    postgres:
      condition: service_healthy
  environment:
    DATABASE_URL: postgres://${POSTGRES_USER:-aeris}:${POSTGRES_PASSWORD}@postgres:5432/${POSTGRES_DB:-aeris}
    RUST_LOG: info,aeris_api=debug,aeris_core=debug
    AERIS_PORT: 8080
    STORAGE_PATH: /var/lib/aeris/storage
  volumes:
    - evidence_storage:/var/lib/aeris/storage
  ports:
    - "8080:8080"
  networks:
    - aeris-net
```

---

# 17. API Dockerfile

`docker/api/Dockerfile`:

```dockerfile
# Multi-stage Rust build
FROM rust:1.85-bookworm AS builder

WORKDIR /usr/src/aeris

# Cache dependencies
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

ENV SQLX_OFFLINE=true
RUN cargo build --release --bin aeris-api

# Minimal runtime image
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

RUN useradd -m -u 1000 aeris && \
    mkdir -p /var/lib/aeris/storage && \
    chown -R aeris:aeris /var/lib/aeris

WORKDIR /app
COPY --from=builder /usr/src/aeris/target/release/aeris-api /app/aeris-api

USER aeris
EXPOSE 8080

CMD ["/app/aeris-api"]
```

---

# 18. Multi-Stage Builds

Production images use multi-stage builds to ensure:

* Zero build toolchains (`cargo`, `rustc`, `gcc`) in runtime images
* Minimal image size ($< 80\text{ MB}$ Debian slim base)
* Minimized CVE attack surface
* Fast container image pull and start times

---

# 19. Worker Container

```yaml
worker:
  build:
    context: .
    dockerfile: docker/worker/Dockerfile
  container_name: aeris-worker
  restart: unless-stopped
  depends_on:
    postgres:
      condition: service_healthy
    ollama:
      condition: service_started
  environment:
    DATABASE_URL: postgres://${POSTGRES_USER:-aeris}:${POSTGRES_PASSWORD}@postgres:5432/${POSTGRES_DB:-aeris}
    OLLAMA_URL: http://ollama:11434
    STORAGE_PATH: /var/lib/aeris/storage
    RUST_LOG: info,aeris_orchestration=debug
  volumes:
    - evidence_storage:/var/lib/aeris/storage
  networks:
    - aeris-net
```

---

# 20. Worker Responsibilities

Workers process asynchronous background pipelines:

* Evidence file ingestion and checksumming
* Multimodal text and PDF extraction
* Candidate claim generation
* Entity resolution
* Temporal interval alignment
* Measurement normalization algebra
* Conflict detection matrix calculation
* Confidence propagation and calibration scoring
* Evaluation execution against ground truth

The API remains non-blocking: it enqueues runs and returns `202 Accepted` immediately.

---

# 21. Frontend Container

`docker/frontend/Dockerfile`:

```dockerfile
FROM node:22-alpine AS build

WORKDIR /app
COPY package*.json ./
RUN npm ci

COPY . .
RUN npm run build

FROM nginx:alpine

COPY docker/frontend/nginx.conf /etc/nginx/conf.d/default.conf
COPY --from=build /app/dist /usr/share/nginx/html

EXPOSE 80
CMD ["nginx", "-g", "daemon off;"]
```

---

# 22. Frontend Configuration

`nginx.conf` proxies API requests directly to the backend container:

```nginx
server {
    listen 80;
    server_name localhost;

    location / {
        root /usr/share/nginx/html;
        index index.html;
        try_files $uri $uri/ /index.html;
    }

    # Proxy REST API
    location /api/ {
        proxy_pass http://api:8080/api/;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
    }

    # Proxy Server-Sent Events (SSE) stream
    location /api/v1/cases/events {
        proxy_pass http://api:8080/api/v1/cases/events;
        proxy_set_header Connection '';
        proxy_http_version 1.1;
        chunked_transfer_encoding off;
        proxy_buffering off;
        proxy_cache off;
    }
}
```

---

# 23. Ollama Service

Local LLM and vision model inference container:

```yaml
ollama:
  image: ollama/ollama:latest
  container_name: aeris-ollama
  restart: unless-stopped
  volumes:
    - ollama_data:/root/.ollama
  networks:
    - aeris-net
```

---

# 24. AI Provider Boundary

AERIS code never calls Ollama APIs directly. It routes via the `AIProvider` trait:

```text
AERIS Core Reasoning
       ↓
  AIProvider (Trait)
       ↓
 ┌─────┴─────────────────────┬───────────────────┐
 ▼                           ▼                   ▼
OllamaProvider       ExternalProvider        MockProvider
```

Testing and development run against `MockProvider` with zero GPU or network dependencies.

---

# 25. GPU Deployment

When deploying on hosts with NVIDIA GPUs:

```yaml
ollama:
  image: ollama/ollama:latest
  deploy:
    resources:
      reservations:
        devices:
          - driver: nvidia
            count: 1
            capabilities: [gpu]
```

---

# 26. GPU Worker Separation

```mermaid
flowchart LR
    Queue["Job Queue"] --> WorkerCPU["CPU Worker: Normalization, ER, Conflict"]
    Queue --> WorkerGPU["GPU Worker: Vision OCR, Local LLM"]
    
    WorkerCPU --> DB[("PostgreSQL")]
    WorkerGPU --> DB
```

Separating GPU workers prevents heavy model calls from delaying fast deterministic joins.

---

# 27. Network Architecture

Docker Compose isolates the application within an internal bridge network:

```yaml
networks:
  aeris-net:
    driver: bridge
```

Services address each other via DNS:

```text
postgres:5432
api:8080
worker
ollama:11434
frontend:80
```

---

# 28. Public Exposure

Only the necessary ingress ports are published to the host:

* `3000:80` (Frontend UI)
* `8080:8080` (API endpoint & OpenAPI docs)

PostgreSQL (`5432`) is bound strictly to `127.0.0.1:5432` for administrative access.

---

# 29. Reverse Proxy

In production, Traefik or Nginx terminates TLS and manages routing:

```mermaid
flowchart LR
    User["HTTPS :443"] --> RP["Reverse Proxy (Traefik / TLS)"]
    RP -->|/| Frontend["Frontend Container :80"]
    RP -->|/api/*| API["API Container :8080"]
```

---

# 30. API Health

Container liveness probe:

```http
GET /health
```

```json
{
  "status": "ok"
}
```

---

# 31. API Readiness

Container readiness probe:

```http
GET /ready
```

Verifies:
* PostgreSQL connection pool is active
* Storage volume is readable and writable
* Core reasoning crates are initialized

```json
{
  "status": "ready",
  "database": "connected",
  "storage": "writable"
}
```

---

# 32. Worker Health

Workers maintain an active heartbeat in the PostgreSQL database:

```sql
UPDATE processing_workers 
SET last_heartbeat = NOW() 
WHERE worker_id = $1;
```

Dead workers are detected after 30 seconds of missed heartbeats.

---

# 33. Container Restart Policy

```yaml
restart: unless-stopped
```

> [!WARNING]
> Restart policies must never be used to mask unhandled crashes. All crash loops must be logged with full stack traces.

---

# 34. Environment Configuration

Managed via `.env`:

```env
POSTGRES_DB=aeris
POSTGRES_USER=aeris
POSTGRES_PASSWORD=generate_a_secure_password_here
DATABASE_URL=postgres://aeris:password@postgres:5432/aeris
AERIS_ENV=development
AI_PROVIDER=ollama
OLLAMA_URL=http://ollama:11434
AERIS_DATASET_SEED=20260930
STORAGE_PATH=/var/lib/aeris/storage
```

---

# 35. Configuration Hierarchy

```mermaid
flowchart TD
    C1["1. Hardcoded Safe Defaults"] --> C2["2. Configuration File (aeris.toml)"]
    C2 --> C3["3. Environment Variables (.env / shell)"]
    C3 --> C4["4. Runtime Command-Line Flags (Highest Precedence)"]
```

Higher layers override lower layers.

---

# 36. Docker Secrets

In swarm/production setups, database credentials use Docker Secrets:

```yaml
secrets:
  db_password:
    file: ./secrets/db_password.txt
```

---

# 37. Object Storage

For the synthetic benchmark, local filesystem storage is mounted directly:

```text
evidence_storage/
└── cases/
    ├── CASE-M001/
    ├── CASE-M002/
    └── ...
```

---

# 38. Object Storage Interface

```rust
use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncWrite};

#[async_trait]
pub trait ObjectStore: Send + Sync {
    async fn put(
        &self,
        key: &str,
        data: Box<dyn AsyncRead + Send + Unpin>,
    ) -> Result<(), StorageError>;

    async fn get(
        &self,
        key: &str,
    ) -> Result<Box<dyn AsyncRead + Send + Unpin>, StorageError>;

    async fn delete(
        &self,
        key: &str,
    ) -> Result<(), StorageError>;
}
```

---

# 39. Local Object Store

Default implementation: `LocalFilesystemObjectStore`. Swappable for `S3ObjectStore` with zero domain code modifications.

---

# 40. Docker Volumes

```yaml
volumes:
  postgres_data:
    name: aeris_postgres_data
  ollama_data:
    name: aeris_ollama_data
  evidence_storage:
    name: aeris_evidence_storage
```

---

# 41. Database Persistence

Data resides exclusively in the `postgres_data` volume. Container recreation or updates will never delete persisted case evidence.

---

# 42. Demo Reset

`scripts/reset-demo.sh`:

```bash
#!/usr/bin/env bash
set -e

echo "=== Resetting AERIS Demo Environment ==="
docker compose down -v
rm -rf datasets/generated/*
docker compose up -d postgres
echo "Waiting for PostgreSQL..."
docker compose exec postgres sh -c 'while ! pg_isready -U aeris; do sleep 1; done'
echo "Reset complete."
```

---

# 43. Demo Seed Process

`scripts/seed-demo.sh`:

```mermaid
flowchart TD
    S1["1. Boot Core Containers"] --> S2["2. Run SQLx Database Migrations"]
    S2 --> S3["3. Trigger Machine Containers (M-001..M-006)"]
    S3 --> S4["4. Register Sources & Ingest Evidence"]
    S4 --> S5["5. Dispatch Pipeline Run"]
    S5 --> S6["6. Await Completion & Compute Metrics"]
    S6 --> S7["7. Launch Browser at http://localhost:3000"]
```

```bash
#!/usr/bin/env bash
set -e

echo "=== Seeding AERIS Demo ==="
docker compose up -d postgres api worker frontend ollama
cargo run --bin aeris-admin -- migrate
docker compose --profile demo up machine-m001 machine-m002 machine-m003 machine-m004 machine-m005 machine-m006
cargo run --bin aeris-admin -- seed-cases
echo "Demo seeded successfully at http://localhost:3000"
```

---

# 44. Synthetic Dataset Initialization

Machine containers generate:
* `telemetry.csv` (1-minute sampled sensor readings)
* `maintenance_report.txt` (Technician work orders)
* `operator_shift_log.txt` (Handoff logs)
* `inspection_photo.png` (High-resolution visual evidence)
* `schematic.pdf` (P&ID equipment diagram)

---

# 45. Deterministic Synthetic Generation

Random seeds are pinned via:

```text
AERIS_DATASET_SEED=20260930
```

Guaranteeing bit-for-bit identical evidence files on every run.

---

# 46. Dataset Manifest

`datasets/generated/manifest.json`:

```json
{
  "dataset": "aeris-factory-v0.1",
  "seed": 20260930,
  "generated_at": "2026-09-30T12:00:00Z",
  "machines": ["M-001", "M-002", "M-003", "M-004", "M-005", "M-006"]
}
```

---

# 47. Startup Dependencies

Startup order:

```text
PostgreSQL (Healthy)
    ↓
API (Ready)
    ↓
Worker (Heartbeat)
    ↓
Frontend (Nginx listening)
```

---

# 48. Compose Health Dependencies

```yaml
depends_on:
  postgres:
    condition: service_healthy
```

Replaces fragile `sleep` statements with verified TCP/socket readiness.

---

# 49. Logging

Structured JSON logging across all containers:

```json
{
  "timestamp": "2026-09-30T20:21:05.124Z",
  "level": "INFO",
  "service": "aeris-worker",
  "case_id": "CASE-M002",
  "run_id": "RUN-001",
  "stage": "conflict_detection",
  "message": "Conflict detected: temperature mismatch 21.7 degC"
}
```

---

# 50. Centralized Logs

View live multi-container logs:

```bash
docker compose logs -f --tail=100
```

---

# 51. Resource Limits

Configured in `docker-compose.yml`:

```yaml
services:
  worker:
    deploy:
      resources:
        limits:
          cpus: "4.0"
          memory: 8192M
        reservations:
          cpus: "1.0"
          memory: 2048M
```

---

# 52. Storage Limits

* Max file upload: $100\text{ MB}$
* Max files per case: 50
* Max uncompressed PDF extraction ratio: $10\times$

---

# 53. Container Security

* Minimal base images (`debian:bookworm-slim`, `alpine`)
* Non-root user execution (`USER aeris`)
* Explicit container isolation via Docker networks
* Secrets never baked into container layers

---

# 54. Non-Root Execution

```dockerfile
RUN groupadd -g 1000 aeris && useradd -u 1000 -g aeris aeris
USER aeris
```

---

# 55. Read-Only Runtime

Containers operate with read-only root filesystems where applicable:

```yaml
read_only: true
tmpfs:
  - /tmp
```

---

# 56. Image Versioning

All images tagged with semver and git commit hash:

```text
aeris-api:0.1.0-abc1234
aeris-worker:0.1.0-abc1234
aeris-frontend:0.1.0-abc1234
```

---

# 57. Build Metadata

`GET /api/v1/system/version`:

```json
{
  "version": "0.1.0",
  "commit": "abc1234",
  "build_time": "2026-09-30T12:00:00Z",
  "environment": "demo"
}
```

---

# 58. Deployment Manifest

Recorded in `deployment.json` at release build:

```json
{
  "aeris_version": "0.1.0",
  "schema_version": "0005_evaluation",
  "dataset_version": "aeris-benchmark-v0.1",
  "ai_provider": "ollama",
  "model_version": "llama3:8b-instruct-q4_K_M"
}
```

---

# 59. CI/CD Pipeline

```mermaid
flowchart LR
    Push["git push"] --> Lint["cargo clippy & fmt"]
    Lint --> Test["cargo test"]
    Test --> Bench["Benchmark Run (Deterministic)"]
    Bench --> Build["Docker Multi-Stage Build"]
    Build --> Scan["Trivy Security Scan"]
    Scan --> Release["Publish Images"]
```

---

# 60. Docker Build Pipeline

```bash
docker compose build --parallel
docker compose up -d
docker compose ps
```

---

# 61. Database Migration During Deployment

Migrations run automatically via `aeris-admin migrate` prior to API and worker initialization.

---

# 62. Migration Command

```bash
cargo run --bin aeris-admin -- migrate
```

---

# 63. Zero-Downtime Future Deployment

Rolling Blue/Green updates using backward-compatible PostgreSQL migrations (additive columns only).

---

# 64. Local Development Workflow

```bash
# 1. Spin up dependencies
docker compose up -d postgres ollama

# 2. Run API locally
cargo run --bin aeris-api

# 3. In another terminal, run frontend
cd apps/workbench && npm run dev
```

---

# 65. Development Compose Profiles

```bash
docker compose --profile benchmark up   # Benchmark evaluator
docker compose --profile ai up          # Ollama local inference
docker compose --profile demo up        # Full synthetic factory demo
```

---

# 66. Demo Topology

```mermaid
flowchart LR
    Browser["Browser"] --> Frontend["Frontend (:3000)"]
    Frontend --> API["API (:8080)"]
    API --> DB[("PostgreSQL (:5432)")]
    API --> Worker["Processing Worker"]
    Worker --> Ollama["Ollama (:11434)"]
    Worker --> Files["Synthetic Evidence Files (M-001..M-006)"]
```

---

# 67. Demo Failure Recovery

If a single generator fails, the orchestrator logs the warning and runs the remaining cases without crashing:

```text
M-001: Success
M-002: Success
M-003: Success
M-004: Generation Error (Flagged)
M-005: Success
M-006: Success
```

---

# 68. Deployment Testing

Full clean environment test:

```bash
./scripts/reset-demo.sh
./scripts/seed-demo.sh
curl -s http://localhost:8080/health | jq .
```

---

# 69. Final Demo Checklist

* [ ] Docker Compose starts cleanly without port conflicts.
* [ ] PostgreSQL passes health check.
* [ ] Database migrations execute successfully.
* [ ] Synthetic machine evidence generates across all 6 cases.
* [ ] Asynchronous worker begins processing pipeline.
* [ ] Frontend loads at `http://localhost:3000`.
* [ ] SSE stream delivers live progress updates.
* [ ] Temperature conflict on M-002 renders with proof card.
* [ ] Telemetry gap on M-004 renders with warning badge.
* [ ] Conveyor ambiguity on M-005 renders with ambiguity badge.
* [ ] Evidence pointers highlight exact PDF pages and sensor rows.
* [ ] Evaluation metrics tab displays F1, ECE, and confusion matrix.

---

# 70. Deployment Invariants

### Invariant 1
Each synthetic machine runs in an isolated container.

### Invariant 2
Original evidence is persisted to named volumes outside container lifecycles.

### Invariant 3
PostgreSQL uses persistent storage volumes.

### Invariant 4
Only frontend and API ports are published to host interfaces.

### Invariant 5
Health checks control service readiness gates.

### Invariant 6
Heavy processing occurs asynchronously in workers, never on HTTP handlers.

### Invariant 7
AI providers are abstracted behind the `AIProvider` trait.

### Invariant 8
Ground-truth benchmarks remain isolated in read-only mounts.

### Invariant 9
All container images are pinned to immutable semver tags.

### Invariant 10
Secrets are never committed to version control.

### Invariant 11
Container restarts do not mutate existing case evidence.

### Invariant 12
The complete demo environment can be reproduced from a clean clone with a single script.

---

# 71. Acceptance Criteria

The deployment system is complete when:

* [ ] `docker-compose.yml` orchestrates the complete AERIS stack.
* [ ] PostgreSQL data persists across `docker compose down` and `up`.
* [ ] API initializes only after PostgreSQL reports healthy.
* [ ] Workers process queued runs asynchronously.
* [ ] Frontend communicates seamlessly with backend API via reverse proxy.
* [ ] AI provider can switch between Ollama, Cloud LLM, and Mock.
* [ ] Six synthetic machine containers run and output evidence files.
* [ ] Health check (`/health`) and readiness check (`/ready`) endpoints return valid status.
* [ ] Structured JSON logs are emitted by all services.
* [ ] Database migrations run automatically.
* [ ] Clean demo reset (`reset-demo.sh`) and seed (`seed-demo.sh`) scripts succeed.
* [ ] Container images use multi-stage builds and run as non-root users.

---

# 72. Final Principle

The AERIS deployment should make the system boring to operate.

That is a compliment.

The desired sequence is:

```text
Clone
  ↓
Configure
  ↓
Build
  ↓
Start
  ↓
Generate Evidence
  ↓
Process
  ↓
Investigate
  ↓
Evaluate
```

No undocumented manual steps should be required to make the demonstration work.

For the Techgium implementation, the recommended architecture remains intentionally simple:

```text
Docker Compose
+
Rust/Axum API
+
Rust Worker
+
PostgreSQL
+
Local Object Storage
+
Optional Ollama
+
Frontend
+
Six isolated synthetic machine containers
```

The architecture can later evolve toward distributed workers, object storage, queues, GPU workers, observability infrastructure, and orchestration platforms without changing the AERIS evidence model.

The deployment layer provides **reproducibility and operational isolation**, while leaving the core evidence reasoning architecture completely independent of infrastructure.
