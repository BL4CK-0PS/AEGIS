-- =============================================================================
-- AERIS v0.1.0 · Initial Database Migration
-- File   : migrations/0001_init.sql
-- Owner  : Kesivarshini (aeris-server, migrations)
-- Source : docs/v0.1/06-contracts.md §"Database: migrations/0001_init.sql"
--
-- NOTE: tower_sessions is intentionally NOT created here.
--       It is created automatically by the tower-sessions-sqlx-store crate at
--       startup. Creating it manually would cause a schema conflict.
--
-- Table creation order follows foreign-key dependencies:
--   operators → (no deps)
--   assets    → (no deps)
--   links     → (no deps)
--   missions  → (no deps)
--   events    → (no deps)
--   evidence  → events
--   incidents → (no deps)
--   decisions → incidents
--   approvals → decisions, operators
--   audit_log → decisions
-- =============================================================================

-- ---------------------------------------------------------------------------
-- 1. operators
--    Human users who can log in and act on decisions.
--    Roles: 'operator' (day-to-day) and 'commander' (high-impact approvals).
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS operators (
    id         TEXT        NOT NULL PRIMARY KEY,
    username   TEXT        NOT NULL UNIQUE,
    pw_hash    TEXT        NOT NULL,                -- argon2id hash
    role       TEXT        NOT NULL
                           CHECK (role IN ('operator', 'commander')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- ---------------------------------------------------------------------------
-- 2. assets
--    Physical or logical nodes in the operational graph (node-a, node-b, …).
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS assets (
    id   TEXT NOT NULL PRIMARY KEY,
    kind TEXT NOT NULL,
    name TEXT NOT NULL
);

-- ---------------------------------------------------------------------------
-- 3. links
--    Communication links between assets (L1, L2).
--    'active' controls whether the link is currently in use.
--    'availability' is a float in [0,1].
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS links (
    id           TEXT             NOT NULL PRIMARY KEY,
    a            TEXT             NOT NULL,   -- source asset id
    b            TEXT             NOT NULL,   -- target asset id
    availability DOUBLE PRECISION NOT NULL
                                  CHECK (availability >= 0.0 AND availability <= 1.0),
    active       BOOLEAN          NOT NULL    DEFAULT TRUE
);

-- ---------------------------------------------------------------------------
-- 4. missions
--    Mission configuration, including capability thresholds stored as JSONB.
--    Example thresholds: {"c_min_nav": 0.85, "c_min_comm": 0.90, "c_min_obs": 0.80}
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS missions (
    id         TEXT        NOT NULL PRIMARY KEY,
    name       TEXT        NOT NULL,
    thresholds JSONB       NOT NULL
);

-- ---------------------------------------------------------------------------
-- 5. events
--    Every raw telemetry / network / auth / sensor event arriving from NATS.
--    UNIQUE(source, seq) is the deduplication key: the same (node, sequence
--    number) pair is never inserted twice, even during partition replay (S7).
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS events (
    id       TEXT             NOT NULL PRIMARY KEY,   -- UUIDv7 from the wire
    source   TEXT             NOT NULL,               -- "node-b" or "node-b/gps"
    seq      BIGINT           NOT NULL,               -- strictly increasing per source
    hlc_l    BIGINT           NOT NULL,               -- Hybrid Logical Clock: physical part
    hlc_c    INTEGER          NOT NULL,               -- Hybrid Logical Clock: counter part
    t        BIGINT           NOT NULL,               -- t_unix_ms from the wire
    type     TEXT             NOT NULL,               -- payload variant name
    payload  JSONB            NOT NULL,               -- full event payload
    sig_ok   BOOLEAN          NOT NULL,               -- Ed25519 signature verified
    replayed BOOLEAN          NOT NULL DEFAULT FALSE, -- true if re-sent after a partition
    received_at TIMESTAMPTZ   NOT NULL DEFAULT now(),

    CONSTRAINT events_source_seq_unique UNIQUE (source, seq)
);

-- Index for time-range queries (most recent events first)
CREATE INDEX IF NOT EXISTS idx_events_t ON events (t DESC);

-- Index for per-source queries
CREATE INDEX IF NOT EXISTS idx_events_source ON events (source);

-- ---------------------------------------------------------------------------
-- 6. evidence
--    Anomaly evidence derived from events by aeris-anomaly.
--    score is a float in [0,1]; higher score = stronger evidence of anomaly.
--    group_id links correlated evidence items (optional).
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS evidence (
    id       TEXT             NOT NULL PRIMARY KEY,   -- evidence ID
    event_id TEXT             NOT NULL
                              REFERENCES events(id) ON DELETE CASCADE,
    entity   TEXT             NOT NULL,               -- "node-b" or "node-b/gps"
    kind     TEXT             NOT NULL,               -- EvidenceKind serde name
    score    DOUBLE PRECISION NOT NULL
                              CHECK (score >= 0.0 AND score <= 1.0),
    group_id TEXT             NULL                    -- optional correlation group
);

-- Index for incident-building queries: find evidence by entity
CREATE INDEX IF NOT EXISTS idx_evidence_entity ON evidence (entity);

-- Index for looking up evidence by event
CREATE INDEX IF NOT EXISTS idx_evidence_event_id ON evidence (event_id);

-- ---------------------------------------------------------------------------
-- 7. incidents
--    An incident tracks a suspected compromise or failure on a specific entity.
--    status is 'open' until resolved.
--    posterior is a JSONB snapshot of the Bayesian hypothesis distribution.
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS incidents (
    id          TEXT        NOT NULL PRIMARY KEY,
    entity      TEXT        NOT NULL,
    opened_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    closed_at   TIMESTAMPTZ NULL,                    -- null while still open
    status      TEXT        NOT NULL DEFAULT 'open'
                             CHECK (status IN ('open', 'closed')),
    posterior   JSONB       NOT NULL                 -- {normal, compromise, sensor_failure, misconfig, environment}
);

-- Index for the most common query: list open incidents
CREATE INDEX IF NOT EXISTS idx_incidents_entity_status ON incidents (entity, status);

-- ---------------------------------------------------------------------------
-- 8. decisions
--    One decision per incident (or per replan cycle).
--    Status follows the lifecycle: COMPUTED → RECOMMENDED → HUMAN_REVIEW →
--      APPROVED / MODIFIED / REJECTED → EXECUTED → VERIFIED
--
--    required_role: the minimum role that may approve this decision.
--      'operator'  for LOW-impact actions
--      'commander' for HIGH-impact actions
--    proposal:      JSONB snapshot of the full Proposal (candidates, scores, etc.)
--    chosen_action: NULL until approved/modified; then the JSONB serialisation
--                   of the chosen Action variant.
--    gate:          'auto', 'human_review' or 'reject' (Gate enum variant name)
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS decisions (
    id              TEXT        NOT NULL PRIMARY KEY,
    incident_id     TEXT        NOT NULL
                                REFERENCES incidents(id) ON DELETE CASCADE,
    status          TEXT        NOT NULL DEFAULT 'COMPUTED'
                                CHECK (status IN (
                                    'COMPUTED',
                                    'RECOMMENDED',
                                    'HUMAN_REVIEW',
                                    'APPROVED',
                                    'MODIFIED',
                                    'REJECTED',
                                    'EXECUTED',
                                    'VERIFIED'
                                )),
    required_role   TEXT        NOT NULL
                                CHECK (required_role IN ('operator', 'commander')),
    proposal        JSONB       NOT NULL,            -- full Proposal struct
    chosen_action   JSONB       NULL,                -- null until approved / modified
    gate            TEXT        NOT NULL,            -- 'auto' | 'human_review' | 'reject'
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Index for the approval SQL: quickly find decisions for an open incident
CREATE INDEX IF NOT EXISTS idx_decisions_incident_status ON decisions (incident_id, status);

-- ---------------------------------------------------------------------------
-- 9. approvals
--    Records every human verdict on a decision.
--    verdict: what the operator/commander did to the decision.
--    reason:  mandatory for 'REJECTED'; optional for others.
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS approvals (
    id          TEXT        NOT NULL PRIMARY KEY,
    decision_id TEXT        NOT NULL
                            REFERENCES decisions(id) ON DELETE CASCADE,
    operator_id TEXT        NOT NULL
                            REFERENCES operators(id) ON DELETE RESTRICT,
    verdict     TEXT        NOT NULL
                            CHECK (verdict IN ('APPROVED', 'MODIFIED', 'REJECTED')),
    reason      TEXT        NULL,                    -- mandatory for REJECTED
    at          TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Index for looking up all approvals for a decision
CREATE INDEX IF NOT EXISTS idx_approvals_decision_id ON approvals (decision_id);

-- ---------------------------------------------------------------------------
-- 10. audit_log
--     Tamper-evident, append-only log of decision records.
--     n:           SERIAL primary key; sequential integer used by the hash chain
--                  and for the GET /api/audit?after={n} cursor.
--     decision_id: the decision this record covers.
--     record:      the raw bytes that were hashed (serialised decision snapshot).
--     hash:        BLAKE3( hash_{n-1} ‖ record ) — chained, not standalone.
--     sig:         Ed25519 signature of hash by the server's private key.
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS audit_log (
    n           SERIAL      NOT NULL PRIMARY KEY,   -- sequential integer, NOT UUID
    decision_id TEXT        NOT NULL
                            REFERENCES decisions(id) ON DELETE RESTRICT,
    record      BYTEA       NOT NULL,               -- raw serialised decision bytes
    hash        BYTEA       NOT NULL,               -- BLAKE3 chained hash
    sig         BYTEA       NOT NULL,               -- Ed25519 server signature of hash
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Index for chain verification scan (ascending order)
CREATE INDEX IF NOT EXISTS idx_audit_log_n ON audit_log (n ASC);
