# 25 — AERIS Development Workflow

## 1. Purpose

This document defines the day-to-day engineering workflow for AERIS.

It covers:

* repository organization and monorepo structure,
* branch management and promotion strategies,
* issue tracking and task decomposition,
* feature development lifecycle,
* peer code review and cross-team contracts,
* deterministic test execution,
* continuous integration and gating,
* database schema migration rules,
* benchmark validation and regression gates,
* release management and demo freeze policies,
* and local development environments.

The goal is to ensure a predictable, reproducible implementation rhythm across all four development tracks without turning the repository into a four-person archaeological site.

---

# 2. Development Lifecycle

Every feature, enhancement, or bug fix follows a strict, observable progression:

```mermaid
flowchart TD
    Req["1. Requirement & Architecture Doc"] --> Des["2. Technical Design & Task Issue"]
    Des --> Impl["3. Implementation on Feature Branch"]
    Impl --> Unit["4. Local Unit & Property Tests"]
    Unit --> Integ["5. Database & API Integration Tests"]
    Integ --> PR["6. Pull Request & Peer Code Review"]
    PR --> CI["7. Automated CI (Lints, Scans, Benchmark Gate)"]
    CI --> Merge["8. Merge into develop / main"]
    Merge --> RC["9. Verified Release Candidate"]
```

```text
Requirement
    ↓
Design
    ↓
Issue
    ↓
Implementation
    ↓
Unit Tests
    ↓
Integration
    ↓
Code Review
    ↓
Benchmark / Regression
    ↓
Merge
    ↓
Release Candidate
```

No major feature or reasoning change is ever pushed directly from an unvetted local experiment to `main`.

---

# 3. Repository Strategy

AERIS is structured as a unified monorepo:

```text
aeris/
```

All four team members collaborate within this single repository.

Components contained in the workspace:

```text
backend (Rust workspace crates)
frontend (React / TypeScript workbench)
benchmark (Automated scoring harness)
datasets (Synthetic generators & isolated ground truth)
docker (Container definitions & Compose topologies)
documentation (Markdown specifications in docs/)
scripts (Seeding, resetting, and deployment tooling)
tests (Unit, integration, contract, and E2E suites)
```

---

# 4. Branch Structure

Git branch taxonomy:

```text
main           (Stable, demo-ready release snapshots)
develop        (Active integration trunk)

feature/*      (New functional capabilities or crates)
fix/*          (Bug fixes and algorithmic corrections)
security/*     (Vulnerability patching and sandboxing)
refactor/*     (Code restructuring with zero contract changes)
docs/*         (Documentation additions and ADRs)
```

Examples:

```text
feature/entity-resolution
feature/conflict-engine
feature/investigation-ui
fix/temporal-overlap
security/file-validation
```

---

# 5. Branch Responsibilities

### `main`
Contains stable, hardened, fully demonstrated releases. Tags such as `v0.1.0-demo` are cut from `main`.

### `develop`
Primary integration branch where feature branches merge after passing automated CI and peer code review.

### `feature/*`
Short-lived branches (1–3 days) dedicated to isolated task cards.

### `fix/*`
Targeted defect remediation branches. Always paired with a regression test.

### `security/*`
Isolated security enhancements, dependency upgrades, or container hardening.

---

# 6. Main Branch Rule

The `main` branch must at all times be:

```text
buildable
testable
deployable
```

> [!CAUTION]
> Knowingly broken code, failing tests, or non-compiling branches must never be merged into `main`. A broken `main` is treated as a team-stopping emergency.

---

# 7. Feature Development

Every feature begins with a defined GitHub/GitLab issue before code is written:

```text
AERIS-142: Implement Fahrenheit to Celsius normalization
```

Issue requirements:
* Clear description of the physical or reasoning problem.
* Explicit inputs, mathematical formulas, and outputs.
* Verifiable acceptance criteria.
* Unit and integration test expectations.
* Dependent crate boundaries.

---

# 8. Issue Example

```markdown
### Title: Implement Fahrenheit to Celsius normalization

### Description:
Measurements expressed in Fahrenheit (°F) must be normalized to canonical
Celsius (°C) prior to comparison, while preserving raw strings in provenance.

### Acceptance Criteria:
- [ ] 32.0°F normalizes to 0.0°C
- [ ] 212.0°F normalizes to 100.0°C
- [ ] 158.0°F normalizes to 70.0°C (M-003 scenario)
- [ ] Original raw value and unit string preserved in MeasurementValue
- [ ] Disallowed units or dimensions rejected cleanly with NormalizationError
- [ ] Property test asserts reversibility: C -> F -> C ≈ C
- [ ] Unit test added to crates/normalization
```

---

# 9. Task Decomposition

Large epics must be broken down into granular, independently reviewable PRs.

Avoid massive PRs like `"Build conflict engine"`.

Instead decompose:

```text
AERIS-201  Candidate pair generation & blocking
AERIS-202  Temporal compatibility gate
AERIS-203  Measurement numerical delta comparator
AERIS-204  5-state conflict classifier
AERIS-205  Structured proof card explanation generator
AERIS-206  Conflict engine unit and regression tests
```

---

# 10. Definition of Ready (DoR)

A task card is ready for engineering implementation when:

* [ ] Problem context and motivation are clear.
* [ ] Expected behavior and formulas are documented.
* [ ] Inputs, outputs, and types are defined.
* [ ] Crate dependencies are declared.
* [ ] Acceptance criteria are unambiguous.
* [ ] Database migration requirements are identified.

---

# 11. Definition of Done (DoD)

A task is considered complete when:

* [ ] Code compiles with zero warnings (`cargo clippy -- -D warnings`).
* [ ] Code conforms to workspace formatting (`cargo fmt --check`).
* [ ] Unit and property tests pass locally and in CI.
* [ ] Integration tests pass against PostgreSQL where applicable.
* [ ] Relevant docs in `docs/` or crate READMEs are updated.
* [ ] Error handling and structured `tracing` logs are implemented.
* [ ] Peer code review approved by at least one maintainer.
* [ ] Benchmark F1 metrics show zero regression.

---

# 12. Commit Strategy

Use small, atomic, conventional commits:

```text
<type>(<scope>): <short imperative summary>
```

Types: `feat`, `fix`, `test`, `docs`, `refactor`, `perf`, `chore`, `security`.

Examples:

```text
feat(entity): add equipment alias surface normalization
feat(conflict): add measurement comparison tolerance gate
fix(temporal): correct Allen interval overlap calculation
test(conflict): add M-002 temperature contradiction regression
docs(api): document SSE progress streaming event envelope
security(ingestion): sanitize file upload paths and reject traversal
```

---

# 13. Commit Rules

> [!WARNING]
> Vague, lazy commit messages like `"update"`, `"wip"`, `"fix stuff"`, `"final"`, or `"final-final2"` are strictly barred. They provide zero diagnostic utility during a git bisect.

---

# 14. Pull Request Requirements

Every pull request must document:
1. **Summary:** Brief description of what changed.
2. **Motivation:** Link to the issue ticket.
3. **Implementation:** Key architectural choices.
4. **Testing Evidence:** Commands run and test outcomes.
5. **Screenshots:** Visual proof for UI changes.
6. **Benchmark Impact:** Confirmation that Macro F1 did not degrade.

---

# 15. Pull Request Template

`.github/pull_request_template.md`:

```markdown
## Summary
<!-- Brief description of changes -->

## Issue Ticket
Closes #<!-- issue number -->

## Implementation Details
<!-- Highlight key traits, structs, or algorithms introduced -->

## Test Coverage
- [ ] Unit tests added / passing (`cargo test -p <crate>`)
- [ ] Integration tests passing (`cargo test --test '*'`)
- [ ] Clippy clean (`cargo clippy --all-targets -- -D warnings`)
- [ ] Formatted (`cargo fmt --check`)

## Benchmark Impact
- [ ] Ran `cargo run --bin aeris-evaluator`
- [ ] Conflict Macro F1: `X.XX` (Target: >= 0.90)

## UI Changes (if applicable)
<!-- Attach screenshots or GIFs of workbench changes -->
```

---

# 16. Code Review Requirements

* Every PR requires approval from at least one other team member.
* Critical reasoning components require sign-off from the subsystem owner:
  - **Entity Resolution:** Track 2 lead sign-off.
  - **Conflict Engine:** Track 3 lead sign-off.
  - **API / Storage:** Track 4 lead sign-off.
  - **Extraction / Ingestion:** Track 1 lead sign-off.

---

# 17. Cross-Team Review Gate (`aeris-core`)

Changes to `crates/aeris-core` (such as `ClaimRecord`, `EvidencePointer`, `EntityRef`, or `ConflictRecord`) alter shared system contracts and require **mandatory approval from all four track leads** before merging.

---

# 18. Core Schema Change Workflow

When modifying a core domain type:

```mermaid
flowchart TD
    Issue["1. Open RFC / Issue detailing change"] --> Mod["2. Update aeris-core struct & enum"]
    Mod --> Serde["3. Update serialization & round-trip tests"]
    Serde --> Crates["4. Update dependent crates (Ingestion, Norm, API, Storage)"]
    Crates --> Mig["5. Add SQLx migration if DB schema affected"]
    Mig --> Test["6. Run cargo test --workspace"]
    Test --> Bench["7. Run aeris-evaluator benchmark"]
    Bench --> PR["8. Cross-team PR Review"]
```

---

# 19. Breaking Changes

Breaking changes (e.g. changing an enum tag, removing a claim field, altering REST endpoints) must be flagged explicitly with `BREAKING CHANGE:` in commit messages and announced to the team channel.

---

# 20. API Versioning

* Production base: `/api/v1`
* Non-breaking additions (e.g. new query parameters, additive response fields) are implemented in `v1`.
* Breaking structural changes require cutting `/api/v2`.

---

# 21. Database Migration Workflow

* Database schemas are versioned exclusively via SQLx migrations in `migrations/`:
  ```text
  migrations/
  ├── 0001_initial_schema.sql
  ├── 0002_source_versions.sql
  ├── 0003_processing_runs.sql
  ├── 0004_confidence.sql
  └── 0005_evaluation.sql
  ```
* Never alter development databases manually through GUI tools without generating a companion migration file.

---

# 22. Migration Testing

Every new migration script must be verified:
1. Applying cleanly to a blank database: `cargo sqlx migrate run`.
2. Applying on top of an existing database seeded with demo data.
3. Asserting foreign key constraints and index creation.

---

# 23. Database Access Boundary

Only `crates/storage` contains SQLx queries and direct connection pool references. API handlers and domain engines call repository interfaces (`ClaimRepository`, `CaseRepository`), never raw SQL.

---

# 24. Local Development Environment

Containerized backend dependencies managed via Docker Compose:

```bash
docker compose up -d postgres ollama
```

Services exposed:
* PostgreSQL 17: `127.0.0.1:5432`
* Ollama: `127.0.0.1:11434`
* Axum API: `127.0.0.1:8080`
* Workbench UI: `127.0.0.1:3000`

---

# 25. Local Environment Configuration

Copy `.env.example` to `.env`:

```env
DATABASE_URL=postgres://aeris:password@127.0.0.1:5432/aeris
STORAGE_PATH=./data/storage
AI_PROVIDER=mock
OLLAMA_URL=http://127.0.0.1:11434
RUST_LOG=info,aeris_api=debug,aeris_core=debug
AERIS_DATASET_SEED=20260930
```

> [!CAUTION]
> Never commit real secrets, private API keys, or operational passwords to Git.

---

# 26. Local Health Check Script

`./scripts/dev-check.sh`:

```bash
#!/usr/bin/env bash
set -e

echo "=== Checking Local Development Health ==="
pg_isready -h 127.0.0.1 -p 5432 -U aeris -d aeris || { echo "PostgreSQL down"; exit 1; }
curl -s http://127.0.0.1:8080/health | grep '"status":"ok"' || { echo "API down"; exit 1; }
echo "All local services healthy."
```

---

# 27. Code Formatting Standards

* Rust:
  ```bash
  cargo fmt --all -- --check
  ```
* Frontend / TypeScript:
  ```bash
  npm run lint
  npm run format:check
  ```

---

# 28. Rust Linting Rules

Enforce strict compiler warnings:

```bash
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Dead code, unnecessary clones, unhandled `Result` types, and raw `unwrap()` calls on runtime data are rejected by CI.

---

# 29. Unit Testing Protocol

Developers must run unit tests before committing:

```bash
# Test entire workspace
cargo test --workspace

# Test specific crate
cargo test -p conflict-engine
```

---

# 30. Integration Testing Protocol

Run integration tests against an isolated test database:

```bash
DATABASE_URL=postgres://aeris:password@127.0.0.1:5432/aeris_test cargo test --test '*'
```

Tests rollback transactions after execution to maintain zero shared state.

---

# 31. Contract Testing

Verify that API responses conform to the OpenAPI TypeScript types shared with `apps/workbench`.

---

# 32. AI Testing Strategy (Mock by Default)

Normal CI pipelines utilize `MockAIProvider` with deterministic JSON fixtures:
* Zero external network dependencies.
* Instantaneous test execution ($< 2\text{s}$).
* 100% reproducible candidate claim generation.

---

# 33. Live AI Integration Testing

Tests requiring live Ollama models live in `tests/live_ai/` and run under an opt-in profile:

```bash
cargo test --test live_ai -- --ignored
```

---

# 34. AI Output Fixtures

Golden fixtures stored in `tests/fixtures/ai/`:
* `valid_observations.json`
* `adversarial_prompt_injection.json`
* `malformed_schema.json`
* `unresolved_entity.json`

---

# 35. Prompt Versioning Workflow

When editing extraction prompts:
1. Create a new versioned file: `crates/extraction/prompts/claim-v4.txt`.
2. Update prompt hash in the extractor metadata.
3. Run extraction unit tests and compare candidate claim yield.
4. Run full benchmark to verify downstream F1 stability.

---

# 36. Entity Resolution Workflow

When adjusting resolver weights or string normalization:
1. Run `cargo test -p entity-resolution`.
2. Run ambiguity tests (`CV-E` vs `CV-E2`).
3. Run benchmark and verify that entity precision and ambiguity detection rates do not drop.

---

# 37. Temporal Engine Workflow

Any adjustment to Allen interval logic requires running:
* Overlapping intervals test
* Disjoint intervals test
* Interval containment test
* Telemetry gap detection test

---

# 38. Measurement Workflow

Unit algebra modifications require verifying:
* Forward and backward conversion symmetry ($C \to F \to C \approx C$).
* Dimension mismatch rejection (`degC` vs `kPa`).
* Tolerance threshold boundary conditions ($|A - B| \le \text{tolerance}$).

---

# 39. Conflict Engine Workflow

Modifications to conflict rules require running the full 7-parameter decision matrix unit tests, followed by the M-002 temperature contradiction regression test.

---

# 40. Benchmark Regression Gate

CI blocks PR merging if `aeris-evaluator` reports:
* Conflict Macro F1 drop $> 0.02$
* Entity Resolution F1 drop $> 0.02$
* Any drop in Provenance Completeness below $100\%$

---

# 41. Benchmark Historical Results

Benchmark metrics are saved as versioned artifacts:

```text
benchmark/results/
├── run-20261001-v0.1/
└── run-20261005-v0.2/
```

---

# 42. Regression Classification

When metrics shift:
* **Improvement:** F1 increases; false alarms decrease.
* **Acceptable Tradeoff:** Slight recall drop ($< 0.01$) for a substantial precision increase ($> 0.05$).
* **Unacceptable Regression:** Any unexplained loss in conflict detection or provenance lineage.

---

# 43. Evidence Provenance Regression

Every claim must link to a valid source pointer. CI enforces:

$$\frac{\text{Claims with Valid Resolving Pointers}}{\text{Total Claims}} = 1.00$$

---

# 44. Frontend Development Workflow

1. Consult OpenAPI / Backend DTOs in `crates/api`.
2. Generate TypeScript types in `apps/workbench/src/types/`.
3. Build components against typed mocks.
4. Integrate with live Axum backend and SSE stream.
5. Add Playwright E2E browser tests.

---

# 45. UI TypeScript Contract Example

```typescript
export interface ConflictRecordDTO {
  id: string;
  case_id: string;
  claim_a: ClaimSummaryDTO;
  claim_b: ClaimSummaryDTO;
  status: 'agreement' | 'conflict' | 'ambiguous' | 'insufficient_evidence' | 'not_comparable';
  reasons: string[];
  confidence: number;
}
```

---

# 46. Frontend UI Testing

Playwright tests verify core user flows:
* Case creation and source registration
* Live SSE progress bar visualization
* Side-by-side proof card opening
* Bounding-box rendering on thermography images

---

# 47. Evidence Viewer Testing

Automated tests assert that clicking an evidence badge scrolls and highlights:
* CSV rows
* PDF page bounding boxes
* Plaintext log snippets

---

# 48. Processing State Visualization

The UI must accurately reflect backend state without desynchronization:
* `Processing` $\to$ Active progress indicator.
* `CompletedWithWarnings` $\to$ Amber warning badge with stage error list.
* `Failed` $\to$ Red error alert with root-cause diagnostic.

---

# 49. Security Workflow

Security-sensitive pull requests require:
* Path traversal exploit testing
* Zip-slip payload checks
* AI prompt injection string resilience checks
* Verification that non-root users (`USER aeris`) run inside containers

---

# 50. Dependency Upgrade Protocol

1. Run `cargo audit` and `npm audit`.
2. Upgrade dependency in `Cargo.toml` or `package.json`.
3. Update lockfile (`Cargo.lock`, `package-lock.json`).
4. Run full test suite and Docker multi-stage builds.

---

# 51. Dependency Pinning

All runtime dependencies and container base images must use strict version pinning or digest hashes. Floating tags like `latest` are prohibited in production Compose files.

---

# 52. Continuous Integration (CI) Pipeline

```mermaid
flowchart TD
    PR["Pull Request Opened"] --> Stage1["Stage 1: Lint & Formatting (fmt, clippy)"]
    Stage1 --> Stage2["Stage 2: Unit & Property Tests (cargo test)"]
    Stage2 --> Stage3["Stage 3: Integration & DB Tests (sqlx::test)"]
    Stage3 --> Stage4["Stage 4: Security & Audit (cargo-audit, trivy)"]
    Stage4 --> Stage5["Stage 5: Multi-Stage Docker Builds"]
    Stage5 --> Stage6["Stage 6: Synthetic Benchmark Gate (aeris-evaluator)"]
    Stage6 --> Pass["CI Passed -> Ready for Review & Merge"]
```

---

# 53. CI Execution Stages

* **Stage 1 (Fast Feedback - 30s):** `cargo fmt --check`, `cargo clippy`, `npm run lint`.
* **Stage 2 (Unit Suite - 45s):** Pure unit tests and property-based tests.
* **Stage 3 (Integration - 90s):** PostgreSQL integration tests and API contracts.
* **Stage 4 (Security - 60s):** `cargo audit`, `trivy` container scanning.
* **Stage 5 (Benchmark Gate - 60s):** 6-machine synthetic evaluation run.

---

# 54. CI Path Optimization

Changes to markdown documentation files in `docs/` trigger only formatting and spell checks, bypassing heavy Docker builds and benchmark suites.

---

# 55. Release Candidate (RC) Preparation

Prior to major milestone demonstrations:
1. Cut release branch `release/v0.1.0`.
2. Run clean end-to-end rehearsal script (`./scripts/seed-demo.sh`).
3. Assert that all 6 machine scenarios pass.
4. Freeze Docker image tags: `aeris-api:0.1.0`, `aeris-worker:0.1.0`.

---

# 56. Versioning Convention

Strict Semantic Versioning (`MAJOR.MINOR.PATCH`):
* `0.1.0`: Hackathon baseline release.
* `0.2.0`: Performance optimizations and advanced calibration.
* `1.0.0`: Production multi-tenant release.

---

# 57. Release Tagging

Git tags are created on `main`:

```bash
git tag -a v0.1.0-demo -m "AERIS Techgium Demo Release"
git push origin v0.1.0-demo
```

---

# 58. Demo Feature Freeze

A strict code freeze goes into effect **48 hours prior to final presentation**:

```mermaid
flowchart LR
    Dev["Active Development"] --> Freeze["Demo Code Freeze (T-48h)"]
    Freeze --> Polish["Stability & Rehearsal Only"]
    Polish --> Demo["Live Techgium Presentation"]
```

---

# 59. Demo Freeze Protocol

During the freeze window:
* **Allowed:** Critical crash fixes, styling alignments, seed script hardening.
* **Prohibited:** New reasoning algorithms, new database migrations, new external dependencies, or major UI redesigns.

---

# 60. Incident Triage Workflow

If an unexpected defect arises:

```text
1. Detect & Isolate   (Capture logs, case_id, and inputs)
2. Reproduce Locally  (Create minimal failing test case in tests/regression/)
3. Root-Cause Analysis(Traverse backward: Finding -> Conflict -> Claim -> Raw Source)
4. Implement Fix      (Smallest possible code delta)
5. Verify Regression  (Failing test now passes)
6. Check Benchmark    (Verify zero unintended side-effects)
7. Fast-Track Deploy  (Merge fix branch)
```

---

# 61. Defect Severity Triage

| Severity | Definition | Target Resolution Time |
| :--- | :--- | :--- |
| **Critical** | Evidence corruption, ground truth leak, panic on 6-machine benchmark | $< 2\text{ hours}$ |
| **High** | Broken provenance link, unit normalization bug, incorrect conflict | $< 6\text{ hours}$ |
| **Medium** | UI styling defect, non-critical API parameter mismatch | $< 24\text{ hours}$ |
| **Low** | Log formatting inconsistency, typo in documentation | Next release |

---

# 62. Reverse Provenance Debugging Method

When an incorrect finding occurs:

```mermaid
flowchart TD
    Finding["Incorrect Finding (e.g. Expected Agreement, Got Conflict)"] --> Step1{"1. Did Claims have same Entity?"}
    Step1 -- No --> Bug1["Bug in Entity Blocking / Resolver"]
    Step1 -- Yes --> Step2{"2. Did Claims share Temporal Overlap?"}
    Step2 -- No --> Bug2["Bug in Temporal Alignment Engine"]
    Step2 -- Yes --> Step3{"3. Were Units converted properly?"}
    Step3 -- No --> Bug3["Bug in Measurement Normalizer"]
    Step3 -- Yes --> Step4{"4. Was Delta evaluated against correct Tolerance?"}
    Step4 -- No --> Bug4["Bug in Conflict Engine Gate / Config"]
    Step4 -- Yes --> Bug5["Bug in AI Candidate Claim Extraction"]
```

Debug from the final finding backward to the source bytes.

---

# 63. Example Debugging Session

Scenario: System flags an unexpected conflict on M-003.
1. Check canonical units: Both claims show Celsius (`degC`).
2. Check values: Claim A is `70.0°C`, Claim B is `70.1°C`.
3. Check tolerance: Configured tolerance is `2.0°C`.
4. Check delta: $|70.0 - 70.1| = 0.1^\circ\text{C} \le 2.0^\circ\text{C}$.
5. Diagnosis: The comparator used `<` instead of `<=` or inverted the boolean flag.
6. Fix: Correct operator in `crates/conflict-engine/src/comparator.rs`.

---

# 64. Entity Resolution Debugging

If `P-002` fails to resolve to `Pump-B`:
* Inspect `normalized_reference` (`"p-002"`).
* Verify alias entry exists in `entity_aliases` table.
* Inspect similarity scores and ensure the resolver threshold is not set $> 0.95$.

---

# 65. Temporal Alignment Debugging

If observations at the same minute fail to overlap:
* Check timezone parsing: Ensure local `UTC+05:30` was normalized to UTC.
* Inspect `TemporalPrecision`: Ensure second-level timestamps are not compared with day-level intervals.

---

# 66. Measurement Normalization Debugging

If `158°F` is compared as `158°C`:
* Check unit tokenizer: Ensure `°F`, `degF`, and `Fahrenheit` map to `Unit::Fahrenheit`.
* Verify algebraic conversion formula: $(F - 32) \times 5/9$.

---

# 67. Development Observability

Developers can inspect pipeline traces by setting:

```bash
RUST_LOG=debug,aeris_orchestration=trace,aeris_conflict_engine=trace
```

Logs display stage-by-stage artifact counts and candidate pair pruning statistics.

---

# 68. Local Debug Flags

```toml
[debug]
trace_candidate_pairs = true
dump_extracted_claims = true
save_intermediate_artifacts = true
```

---

# 69. Reproducible Bug Report Format

Every bug report must include:
* Case ID and Processing Run ID
* Pinned dataset seed (`AERIS_DATASET_SEED=20260930`)
* Software commit hash
* Expected vs. observed evidence relationship
* Reproduction command (`cargo test --test issue_XXX`)

---

# 70. Documentation as Code

Documentation in `docs/` is treated as source code:
* Any change to an API endpoint requires updating `13-api-and-service-contracts.md`.
* Any database schema change requires updating `15-storage-and-database.md`.
* Documentation PRs merge alongside code PRs.

---

# 71. Architecture Decision Records (ADR)

Significant architectural shifts are documented in `docs/adr/`:

```text
docs/adr/
├── 0001-rust-axum-backend.md
├── 0002-deterministic-conflict-engine.md
├── 0003-postgresql-dual-storage.md
├── 0004-isolated-ground-truth.md
└── 0005-weakest-link-confidence.md
```

---

# 72. ADR Markdown Template

```markdown
# ADR-000X: Title of Decision

## Status
Accepted / Proposed / Superseded

## Context
What engineering problem or bottleneck prompted this decision?

## Decision
What architectural choice was adopted?

## Consequences
What are the positive outcomes and tradeoffs?

## Alternatives Considered
Why were competing approaches rejected?
```

---

# 73. Architecture Change Process

Proposing a major change (e.g. replacing SQLx or adding a vector store) requires:
1. Submitting an ADR PR.
2. Demonstrating why existing contracts fail.
3. Quantifying benchmark and latency impacts.
4. Unanimous team agreement.

---

# 74. Performance Profiling Workflow

Optimize only with empirical profiler data:
* Use `cargo-flamegraph` for CPU bottlenecks.
* Use `EXPLAIN ANALYZE` for PostgreSQL query plans.
* Use Chrome DevTools for UI rendering frame rates.

---

# 75. Benchmark Performance Testing

Run multi-case stress profiles:

```bash
cargo run --release --bin aeris-benchmark -- --cases 50
```

Assert that execution scales sub-quadratically and memory RSS remains bounded.

---

# 76. Performance Regression Policy

A performance optimization that decreases Conflict F1 by even $0.01$ is **immediately rejected**. Correctness and forensic integrity strictly supersede speed.

---

# 77. Daily Developer Rhythm

```text
1. git checkout develop && git pull
2. Check assigned issue card
3. git checkout -b feature/issue-XXX
4. Write failing unit test
5. Implement feature logic
6. cargo clippy & cargo fmt
7. git commit -m "feat(scope): implement..."
8. git push -u origin feature/issue-XXX
9. Open Pull Request
```

---

# 78. Continuous Team Integration Rhythm

```mermaid
flowchart TD
    T1["Track 1: Ingestion & Multimodal"] --> Branch["Integration Branch (develop)"]
    T2["Track 2: Entity & Temporal"] --> Branch
    T3["Track 3: Reasoning & Normalization"] --> Branch
    T4["Track 4: API, Storage & UI"] --> Branch
    
    Branch --> DailyCI["Nightly End-to-End Benchmark Run"]
    DailyCI --> Green["Consistently Green Trunk"]
```

---

# 79. Integration Frequency Rule

Branches must merge into `develop` at least once every 48 hours. Long-lived feature branches that drift for weeks are prohibited.

---

# 80. Weekly Milestone Target

Every Friday afternoon:
* The full 6-machine benchmark executes end-to-end.
* The team reviews the live UI workbench together.
* Any integration mismatches are triaged immediately.

---

# 81. Pre-Demo Rehearsal Workflow

Twenty-four hours prior to evaluation:
1. Run `./scripts/reset-demo.sh` on clean demo hardware.
2. Run `./scripts/seed-demo.sh`.
3. Verify that the 6 scenarios load cleanly in the browser.
4. Execute dry-run 3-minute and 5-minute presentation scripts.

---

# 82. Live Demo Script Rehearsal Steps

1. Open `http://localhost:3000`.
2. Select `CASE-047`.
3. Highlight live SSE processing.
4. Drill down on M-002 temperature contradiction ($72.3^\circ\text{C}$ vs $94.0^\circ\text{C}$).
5. Inspect M-003 unit normalization ($158.0^\circ\text{F} \equiv 70.0^\circ\text{C}$).
6. Showcase M-004 telemetry dropout gap.
7. Showcase M-005 entity ambiguity badge.
8. Showcase M-006 visual thermography hotspot.
9. Open Benchmark dashboard showing $0.914\text{ Macro F1}$.

---

# 83. Demo Backup Package

Packaged on offline USB media:
* Pre-built Docker container images (`docker save`).
* Seed database dump (`demo_seed.sql`).
* High-resolution demonstration video walkthrough.
* PDF slides of proof cards and architecture diagrams.

---

# 84. Final Development Checklist

```text
[ ] Branching model active (main, develop, feature/*)
[ ] Conventional commits enforced
[ ] Clippy (-D warnings) and cargo fmt active in CI
[ ] Core domain contracts pinned in aeris-core
[ ] PostgreSQL migrations managed exclusively via SQLx
[ ] Mock AI provider used for deterministic CI testing
[ ] Benchmark evaluation gate active in CI
[ ] Security checks and path traversal tests active
[ ] Side-by-side proof cards verified on workbench UI
[ ] Demo reset and seed scripts tested and verified
```

---

# 85. Final Principle

AERIS development follows one core engineering imperative:

> **Every meaningful commit must leave the system more reproducible, more testable, or more explainable than before.**

The development loop is disciplined and transparent:

$$\text{Design} \implies \text{Test} \implies \text{Implement} \implies \text{Benchmark} \implies \text{Review} \implies \text{Document}$$

For every change, the team must ask:

* Did this change alter what evidence means?
* Did it alter entity identity?
* Did it alter temporal relationships?
* Did it alter measurement algebra?
* Did it alter conflict classification?
* Did it preserve complete provenance?

If the answer is yes, the benchmark and test suite must validate the change before it merges.

This workflow guarantees that AERIS is a robust, forensic evidence reasoning platform built with shared discipline, predictable velocity, and unshakeable engineering integrity.
