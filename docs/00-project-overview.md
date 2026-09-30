# AERIS

## Evidence Alignment and Reasoning Intelligence System

### 1. Project Overview

AERIS is a multimodal evidence reasoning engine designed to transform heterogeneous information sources into a unified, traceable, and inspectable representation of real-world entities, events, measurements, and claims.

Modern operational systems generate evidence across multiple formats, including text documents, images, structured tables, and time-series sensor data. These sources frequently describe the same entity or event using different names, units, formats, and timestamps. They may also contain incomplete or contradictory observations.

AERIS addresses this problem by creating a common evidence representation in which every claim retains its provenance, temporal context, confidence, and classification as either directly extracted or inferred.

The system performs:

1. Multimodal evidence ingestion
2. Structured claim extraction
3. Cross-modal entity resolution
4. Temporal alignment
5. Measurement and unit normalization
6. Evidence comparison
7. Conflict and agreement detection
8. Confidence and uncertainty assessment
9. Evidence-level inspection and traceability

The central design principle of AERIS is:

> **AI interprets evidence; deterministic reasoning validates relationships between evidence.**

AI models are used where interpretation of unstructured or multimodal information is required. Deterministic components are responsible for validating entity relationships, temporal overlap, measurement comparability, normalization, and conflict conditions.

---

## 2. Problem

Information describing a real-world system is rarely contained in a single source.

For example, an industrial machine may be represented simultaneously through:

* A maintenance report
* Technician observations
* Inspection images
* Structured maintenance records
* Temperature telemetry
* Vibration telemetry
* Event logs

These sources may use different identifiers and representations for the same entity.

For example:

```text
Pump-B
Pump B
P-002
Pump_02
```

The same measurement may also appear using different units:

```text
72 °C
161.6 °F
```

Furthermore, observations may refer to different points in time or overlapping time intervals.

A system that compares these observations without considering identity, time, units, and provenance can incorrectly identify conflicts or incorrectly merge contradictory information.

AERIS therefore treats evidence reconciliation as a structured reasoning problem rather than a simple similarity or retrieval problem.

---

## 3. Core Concept

AERIS converts raw multimodal evidence into structured claims.

A simplified pipeline is:

```text
Text / Images / Tables / Sensors
              │
              ▼
       Evidence Extraction
              │
              ▼
        Claim Records
              │
       ┌──────┼──────┐
       ▼      ▼      ▼
    Entity   Time   Measurement
    Linking Alignment Normalization
       │      │      │
       └──────┼──────┘
              ▼
       Evidence Comparison
              │
       ┌──────┼─────────────┐
       ▼      ▼             ▼
   Agreement Conflict    Ambiguity
              │
              ▼
       Confidence & Evidence
              │
              ▼
       Investigation Interface
```

The system does not discard contradictory evidence. Instead, conflicting claims remain independently represented and are linked through an explicit conflict relationship.

---

## 4. Evidence Model

The fundamental unit of information in AERIS is the **ClaimRecord**.

A claim represents an observation or derived statement about an entity, event, measurement, or attribute.

Conceptually:

```text
Claim
 ├── Subject
 ├── Predicate
 ├── Object / Value
 ├── Temporal Context
 ├── Provenance
 ├── Claim Type
 ├── Confidence
 └── Status
```

A claim may be:

### Extracted

Directly supported by a source.

Example:

```text
Sensor reported:
Pump-B temperature = 94 °C
```

### Inferred

Derived from one or more other claims.

Example:

```text
Pump-B experienced a temperature anomaly.
```

AERIS maintains this distinction so that derived conclusions cannot be confused with direct observations.

---

## 5. Provenance

Every claim must retain a pointer to the evidence from which it originated.

Depending on the source, an evidence pointer may refer to:

* PDF page and text range
* Image region
* Table row and column
* CSV record
* Sensor timestamp
* Structured data field

The provenance model allows users to inspect not only what AERIS concluded, but also the evidence supporting the conclusion.

---

## 6. Entity Resolution

Different sources may use different identifiers for the same real-world entity.

AERIS attempts to resolve these references into canonical entities while preserving uncertainty.

For example:

```text
Pump-B
Pump B
P-002
Pump_02
        │
        ▼
     Pump-B
```

When available evidence is insufficient to determine identity, AERIS represents the relationship as ambiguous rather than forcing an incorrect merge.

---

## 7. Temporal Alignment

Claims are associated with temporal information.

AERIS considers:

* Timestamps
* Time intervals
* Time zones
* Temporal precision
* Overlapping observation windows
* Event ordering

Temporal alignment prevents unrelated observations from being incorrectly treated as contradictory.

Two claims should only be considered potential conflicts when their temporal contexts are sufficiently comparable.

---

## 8. Measurement Normalization

Measurements from different sources may use different units or representations.

AERIS converts comparable measurements into canonical units before evaluating them.

For example:

```text
158 °F
    │
    ▼
≈ 70 °C
```

Therefore, numerical differences alone do not automatically indicate a conflict.

---

## 9. Conflict Detection

AERIS classifies relationships between claims rather than simply comparing their values.

A potential conflict requires conditions such as:

```text
Same Entity
      AND
Same Attribute
      AND
Temporal Overlap
      AND
Comparable Units
      AND
Compatible Semantic Meaning
      AND
Difference Beyond Tolerance
```

Possible outcomes include:

* **Agreement**
* **Conflict**
* **Ambiguous**
* **Insufficient Evidence**
* **Not Comparable**

Contradictory evidence is never silently averaged away.

---

## 10. Confidence and Uncertainty

AERIS treats uncertainty as an explicit property of the evidence.

Confidence may be associated with different stages, including:

* Extraction confidence
* Entity-resolution confidence
* Temporal-alignment confidence
* Source reliability
* Inference confidence

The system is designed to evaluate confidence against known ground-truth cases rather than treating model-generated confidence values as inherently reliable.

---

## 11. Investigation Interface

AERIS provides an interface through which users can inspect:

* Cases
* Entities
* Events
* Claims
* Timelines
* Conflicts
* Ambiguities
* Source evidence
* Confidence information

A user should be able to move from a detected conflict directly to the supporting source evidence.

For example:

```text
Conflict
   │
   ├── Claim A
   │     └── sensor.csv → Row 481
   │
   └── Claim B
         └── report.pdf → Page 7
```

This creates an auditable chain between the system's reasoning and the original evidence.

---

## 12. Evaluation

AERIS will be evaluated using controlled multimodal cases containing:

* Normal agreement
* Genuine conflicts
* Unit variations
* Missing evidence
* Ambiguous entity references
* Cross-modal disagreement
* Temporal differences

Evaluation will measure areas including:

* Entity-linking performance
* Temporal-alignment accuracy
* Conflict-detection precision and recall
* Confidence calibration
* Robustness to missing and ambiguous evidence

Ground-truth data will be maintained separately from the inference pipeline and used only for evaluation.

---

## 13. Design Philosophy

AERIS follows five core principles.

### 1. Preserve Evidence

Conflicting observations remain visible.

### 2. Preserve Provenance

Every claim should be traceable to its source.

### 3. Respect Time

Claims should only be compared when their temporal context is relevant.

### 4. Represent Uncertainty

Ambiguity and insufficient evidence are valid system outcomes.

### 5. Separate Interpretation from Verification

AI models interpret multimodal evidence, while deterministic components validate relationships and reasoning conditions.

---

## 14. Target Outcome

AERIS aims to provide a trustworthy foundation for multimodal AI systems where users need to understand not only the generated conclusion, but also:

* What was observed
* Which entity it refers to
* When it occurred
* Where the evidence came from
* How the evidence relates to other observations
* Whether sources agree or conflict
* How certain the system is
* What remains unknown

The objective is therefore not simply to produce an answer, but to produce an **inspectable evidence-backed representation of the situation**.
