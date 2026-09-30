# AERIS System Requirements

## 1. Purpose

This document defines the functional and non-functional requirements for the AERIS (Evidence Alignment and Reasoning Intelligence System).

The requirements establish what the system must ingest, represent, process, reason about, expose, and evaluate.

All subsequent implementation and testing should be traceable to these requirements.

---

## 2. System Scope

AERIS shall provide an evidence reasoning pipeline capable of processing heterogeneous multimodal sources and producing a unified, provenance-aware representation of entities, events, measurements, and claims.

The system shall support:

```text
Text
Images
Structured Data
Time-Series Data
```

and shall provide:

```text
Claim Extraction
Provenance Tracking
Entity Resolution
Temporal Alignment
Normalization
Conflict Detection
Confidence Assessment
Evidence Inspection
Evaluation
```

---

## 3. Functional Requirements

### FR-001: Case Management

The system shall support creation and management of investigation cases.

Each case shall have:

* Unique case identifier
* Case metadata
* Associated sources
* Associated entities
* Associated events
* Associated claims
* Associated conflicts

Example:

```text
CASE-047
```

---

### FR-002: Multimodal Source Ingestion

The system shall accept evidence from multiple source types.

Supported initial formats shall include:

```text
PDF
TXT
CSV
JSON
PNG
JPEG
Time-Series Data
```

Each source shall be registered before processing.

---

### FR-003: Source Registration

AERIS shall create a `SourceRecord` for every ingested source.

The source record shall contain sufficient metadata to identify:

* Source identifier
* Case identifier
* Source type
* Original location
* File metadata
* Processing metadata
* Optional source reliability information

---

### FR-004: Text Extraction

The system shall extract relevant information from textual sources.

The extraction process shall identify candidate:

* Entities
* Events
* Attributes
* Measurements
* Temporal references
* Relationships

---

### FR-005: Image Processing

The system shall support image-based evidence.

The system shall be capable of extracting or representing relevant observations from images using appropriate OCR or vision-based processing.

Image-derived evidence shall retain a reference to the originating image and, where available, the relevant image region.

---

### FR-006: Structured Data Processing

The system shall process structured sources such as CSV files, tables, and JSON records.

Structured evidence shall retain sufficient information to identify the original:

* File
* Record
* Row
* Column
* Field

---

### FR-007: Time-Series Processing

The system shall process time-series observations.

Each observation shall retain:

* Entity reference where available
* Measurement type
* Value
* Unit
* Timestamp
* Source reference

The system shall support sequences of observations rather than treating each observation as an isolated text statement.

---

## 4. Evidence Representation Requirements

### FR-008: Unified Evidence Model

AERIS shall represent heterogeneous observations using a common internal evidence model.

The model shall support:

```text
Entities
Events
Claims
Measurements
Temporal Context
Provenance
Confidence
```

---

### FR-009: ClaimRecord

The system shall represent individual observations using a `ClaimRecord`.

A claim shall contain, at minimum:

```text
Claim ID
Subject
Predicate
Object / Value
Temporal Context
Provenance
Claim Type
Confidence
Status
```

---

### FR-010: Extracted and Inferred Claims

The system shall distinguish between:

#### Extracted claims

Claims directly supported by source evidence.

#### Inferred claims

Claims derived from other evidence through system reasoning.

The two categories shall not be silently merged.

---

## 5. Provenance Requirements

### FR-011: Claim-Level Provenance

Every claim shall maintain one or more evidence pointers to its supporting source.

A claim shall not lose its source relationship during processing.

---

### FR-012: Text Provenance

Text evidence shall support source references such as:

```text
Document
Page
Paragraph
Character Range
```

where available.

---

### FR-013: Image Provenance

Image evidence shall support:

```text
Image
Bounding Region
```

where a relevant region can be identified.

---

### FR-014: Structured Data Provenance

Structured evidence shall support:

```text
File
Row
Column
Field
```

references.

---

### FR-015: Time-Series Provenance

Time-series evidence shall maintain references to the relevant source and timestamp.

---

## 6. Entity Resolution Requirements

### FR-016: Canonical Entities

The system shall maintain canonical representations of identified entities.

Each entity may contain:

* Entity identifier
* Canonical name
* Entity type
* Known aliases
* Resolution confidence

---

### FR-017: Cross-Source Entity Linking

The system shall identify potentially equivalent entity references across different sources.

Example:

```text
Pump-B
Pump B
P-002
Pump_02
```

may be linked to the same canonical entity when sufficient evidence exists.

---

### FR-018: Entity Ambiguity

The system shall explicitly represent unresolved or ambiguous entity relationships.

The system shall not automatically merge entities when the available evidence is insufficient.

---

## 7. Temporal Requirements

### FR-019: Temporal Context

Claims and events shall support temporal information.

The temporal representation shall support:

* Point timestamps
* Time intervals
* Approximate timestamps
* Time zones
* Temporal precision

---

### FR-020: Temporal Normalization

The system shall normalize compatible timestamps into a common representation.

---

### FR-021: Temporal Alignment

The system shall determine temporal relationships between claims and events.

Supported relationships should include:

```text
Before
After
Overlaps
Contains
Contained By
Concurrent
Unknown
```

---

### FR-022: Temporal Conflict Filtering

The conflict engine shall not classify claims as contradictory solely because their values differ.

Temporal compatibility shall be evaluated before a conflict is established.

---

## 8. Measurement Requirements

### FR-023: Measurement Representation

Measurements shall contain:

```text
Value
Unit
Measurement Type
Entity
Timestamp / Temporal Context
```

where applicable.

---

### FR-024: Unit Normalization

The system shall normalize compatible measurement units before comparison.

Example:

```text
158 °F
≈ 70 °C
```

---

### FR-025: Tolerance

The system shall support configurable tolerances for measurements where exact equality is not appropriate.

Example:

```text
Expected:
70 °C

Observed:
70.2 °C

Tolerance:
±1 °C

Result:
Agreement
```

---

## 9. Evidence Reasoning Requirements

### FR-026: Evidence Comparison

The system shall compare claims only after relevant identity, temporal, and measurement relationships have been established.

---

### FR-027: Agreement Detection

The system shall identify compatible observations as agreement when they satisfy the required comparison conditions.

---

### FR-028: Conflict Detection

The system shall identify genuine conflicts when comparable claims contradict each other beyond the configured tolerance.

A conflict should consider:

```text
Entity Identity
Attribute Identity
Temporal Compatibility
Measurement Comparability
Semantic Compatibility
Value Difference
Tolerance
```

---

### FR-029: Conflict Preservation

The system shall preserve all claims participating in a conflict.

The system shall not:

* Average conflicting values
* Delete one source
* Silently select a preferred claim
* Hide the disagreement

---

### FR-030: Evidence Status

The system shall support explicit evidence classifications:

```text
Agreement
Conflict
Ambiguous
Insufficient Evidence
Not Comparable
```

---

## 10. Missing Evidence Requirements

### FR-031: Missing Data Detection

The system shall identify missing or unavailable evidence when the absence affects the ability to evaluate a claim or event.

---

### FR-032: No Unsupported Inference

The system shall not generate a definitive observation solely because expected evidence is missing.

For example:

```text
Missing sensor data
```

shall not automatically become:

```text
Machine operating normally
```

---

## 11. Confidence Requirements

### FR-033: Confidence Representation

The system shall represent confidence associated with relevant processing stages.

Potential dimensions include:

```text
Extraction Confidence
Entity Resolution Confidence
Temporal Confidence
Source Reliability
Inference Confidence
```

---

### FR-034: Confidence Traceability

Confidence values shall be associated with the corresponding claim, entity, relationship, or inference.

---

### FR-035: Calibration Evaluation

The system shall evaluate confidence estimates against known ground-truth outcomes.

Evaluation shall support metrics such as:

```text
Brier Score
Expected Calibration Error
```

where applicable.

---

## 12. Investigation Interface Requirements

### FR-036: Case Dashboard

The interface shall provide an overview of a case, including relevant counts and evidence status.

Example:

```text
Entities
Events
Claims
Conflicts
Ambiguities
Missing Evidence
```

---

### FR-037: Claim Inspection

Users shall be able to inspect an individual claim and view:

* Subject
* Predicate
* Value
* Temporal context
* Confidence
* Claim type
* Supporting evidence

---

### FR-038: Conflict Inspection

Users shall be able to inspect both sides of a detected conflict.

The interface shall explain the conditions that caused the conflict classification.

Example:

```text
Same entity            ✓
Same attribute         ✓
Temporal overlap       ✓
Comparable units       ✓
Difference > tolerance ✓
```

---

### FR-039: Evidence Navigation

Users shall be able to navigate from a claim to its supporting source evidence.

---

### FR-040: Timeline Visualization

The system shall provide a temporal view of relevant events and claims.

---

### FR-041: Entity Inspection

Users shall be able to inspect:

* Canonical entity
* Aliases
* Linked evidence
* Resolution confidence
* Ambiguous relationships

---

## 13. Evaluation Requirements

### FR-042: Ground Truth

The project shall maintain a separate ground-truth dataset for evaluation.

Ground truth shall not be used as input to the inference pipeline.

---

### FR-043: Benchmark Scenarios

The benchmark shall include cases representing:

```text
Normal Agreement
Genuine Conflict
Unit Variation
Missing Data
Entity Ambiguity
Cross-Modal Disagreement
Temporal Variation
```

---

### FR-044: Entity Resolution Evaluation

Entity linking performance shall be evaluated against known ground truth.

Appropriate metrics may include:

```text
Precision
Recall
F1 Score
```

---

### FR-045: Conflict Detection Evaluation

Conflict detection shall be evaluated using known conflict and non-conflict cases.

Metrics shall include:

```text
Precision
Recall
F1 Score
```

---

### FR-046: Temporal Evaluation

Temporal alignment shall be evaluated using cases with known temporal relationships.

---

### FR-047: Robustness Evaluation

The system shall be tested under:

* Missing sources
* Ambiguous names
* Different units
* Incomplete timestamps
* Contradictory observations
* Extraction uncertainty

---

## 14. Non-Functional Requirements

### NFR-001: Traceability

All system conclusions shall remain traceable to their supporting evidence.

---

### NFR-002: Auditability

The system shall maintain sufficient information to reconstruct how a claim or conflict was produced.

---

### NFR-003: Modularity

The major processing components shall be independently replaceable.

Core components include:

```text
Ingestion
Extraction
Normalization
Entity Resolution
Temporal Engine
Conflict Engine
Confidence Engine
Storage
API
Interface
```

---

### NFR-004: Reproducibility

Given the same source data, configuration, and model version, the system should produce reproducible reasoning results where deterministic processing is expected.

---

### NFR-005: Extensibility

New source formats and extraction providers should be addable without redesigning the core evidence model.

---

### NFR-006: Reliability

Failure in one source or processing component should not invalidate unrelated evidence.

---

### NFR-007: Security

The system shall protect uploaded evidence and restrict access to case data according to the deployment environment.

---

### NFR-008: Performance

The system should process benchmark cases within a practical time suitable for interactive investigation.

Exact performance targets shall be established after baseline implementation and profiling.

---

### NFR-009: Observability

The system should expose sufficient logs and processing status to diagnose failures during ingestion and reasoning.

---

## 15. System Constraints

AERIS shall operate under the following design constraints:

1. Raw evidence must remain preserved.
2. Conflicting evidence must not be silently discarded.
3. Entity resolution must support ambiguity.
4. Temporal context must participate in conflict reasoning.
5. Comparable measurements must be normalized before comparison.
6. Extracted and inferred information must remain distinguishable.
7. Ground truth must remain separate from inference.
8. AI-generated extraction must not be treated as unquestionable truth.
9. Critical conflict decisions should be explainable through explicit reasoning conditions.
10. Every major conclusion should remain traceable to source evidence.

---

## 16. Acceptance Criteria

The initial AERIS implementation shall be considered functionally complete when it can successfully demonstrate the following:

### Scenario 1: Agreement

```text
72 °C
158 °F

→ Agreement after normalization
```

### Scenario 2: Conflict

```text
72.3 °C
94 °C

→ Genuine Conflict
```

### Scenario 3: Ambiguity

```text
CV-E
CV-E2

→ Ambiguous Entity
```

### Scenario 4: Missing Evidence

```text
09:25 → 09:29
No telemetry

→ Insufficient Evidence
```

### Scenario 5: Cross-Modal Disagreement

```text
Text:
Component intact

Image:
Component appears damaged

→ Cross-Modal Conflict / Disagreement
```

Each result must remain linked to its supporting source evidence.

---

## 17. Requirements Traceability

The implementation should maintain a traceability relationship:

```text
Requirement
     ↓
Component
     ↓
Implementation
     ↓
Test Case
     ↓
Evaluation Result
```

Example:

```text
FR-028 Conflict Detection
        ↓
Conflict Engine
        ↓
conflict-engine/src/
        ↓
CASE-002
        ↓
Conflict F1
```

This structure will later make it easier to demonstrate that the implemented system directly addresses the problem requirements rather than merely presenting a collection of unrelated features.

---

## 18. Requirements Summary

AERIS must ultimately provide:

```text
Multimodal Ingestion
        ↓
Structured Evidence
        ↓
Claim Representation
        ↓
Provenance
        ↓
Entity Resolution
        ↓
Temporal Alignment
        ↓
Measurement Normalization
        ↓
Conflict / Agreement Reasoning
        ↓
Confidence & Uncertainty
        ↓
Evidence Inspection
        ↓
Benchmark Evaluation
```

These requirements form the baseline against which the AERIS architecture and implementation will be developed and evaluated.

---

**Next document: `04-architecture.md`.** That is where we lock the actual technical architecture, component boundaries, data flow, Rust crate structure, AI boundary, PostgreSQL layer, APIs, and deployment topology.
