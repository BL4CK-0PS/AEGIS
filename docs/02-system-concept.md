# AERIS System Concept

## 1. Solution Overview

AERIS is designed as a provenance-aware multimodal evidence reasoning system.

The system accepts heterogeneous evidence from text documents, images, structured data, and time-series sources. Instead of treating each source independently, AERIS transforms the information into a common representation of entities, events, measurements, and claims.

These claims are then progressively processed through entity resolution, temporal alignment, normalization, and evidence reasoning.

The overall objective is to produce an evidence state in which every important observation can be traced to its source and compared with other observations without losing the original evidence.

The high-level processing flow is:

```text
┌─────────────────────────────────────────────────────────┐
│                    MULTIMODAL SOURCES                   │
│                                                         │
│  Documents   Images   Tables   CSV   Sensor Data       │
└──────────────────────────┬──────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────┐
│                    EVIDENCE INGESTION                   │
│                                                         │
│  Parsing • OCR • Metadata • Time-Series Processing     │
└──────────────────────────┬──────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────┐
│                  CLAIM EXTRACTION                       │
│                                                         │
│  Entities • Events • Measurements • Observations       │
└──────────────────────────┬──────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────┐
│                UNIFIED EVIDENCE MODEL                  │
│                                                         │
│  Claims • Provenance • Temporal Context • Confidence   │
└──────────────────────────┬──────────────────────────────┘
                           │
              ┌────────────┼────────────┐
              ▼            ▼            ▼
       Entity Resolution  Temporal    Normalization
                          Alignment
              │            │            │
              └────────────┼────────────┘
                           ▼
┌─────────────────────────────────────────────────────────┐
│                  EVIDENCE REASONING                     │
│                                                         │
│  Agreement • Conflict • Ambiguity • Missing Evidence   │
└──────────────────────────┬──────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────┐
│               CONFIDENCE & EVALUATION                  │
└──────────────────────────┬──────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────┐
│              INVESTIGATION INTERFACE                   │
│                                                         │
│  Claims • Timeline • Conflicts • Evidence • Entities   │
└─────────────────────────────────────────────────────────┘
```

---

## 2. Core System Principle

AERIS follows a hybrid AI and deterministic reasoning architecture.

The core principle is:

> **AI interprets evidence; deterministic software validates evidence relationships.**

AI models are useful for interpreting unstructured information.

For example:

```text
Maintenance Report
        │
        ▼
      AI Model
        │
        ▼
Candidate observation:
Pump-B temperature = 94°C
```

However, the AI model should not independently determine whether this contradicts another source.

Instead:

```text
Candidate Claims
      │
      ▼
Canonical Representation
      │
      ▼
Entity Resolution
      │
      ▼
Temporal Alignment
      │
      ▼
Unit Normalization
      │
      ▼
Deterministic Conflict Engine
```

This separation makes the reasoning process more reproducible and inspectable.

---

## 3. Evidence Ingestion Layer

The ingestion layer is responsible for accepting heterogeneous input sources.

Supported source categories include:

### Documents

```text
PDF
TXT
Markdown
JSON
```

### Images

```text
JPEG
PNG
Inspection photographs
Scanned documents
```

### Structured Data

```text
CSV
Tables
Structured JSON
Component registries
```

### Time-Series

```text
Temperature
Pressure
Vibration
Machine state
Event streams
```

Each source is registered as a `SourceRecord`.

The source record provides the identity and metadata required to maintain provenance throughout the processing pipeline.

---

## 4. Multimodal Extraction

After ingestion, AERIS extracts candidate information from each source.

Different modalities require different extraction mechanisms.

```text
Text
 ↓
NLP / LLM extraction

Image
 ↓
OCR / Vision extraction

CSV
 ↓
Structured parser

Sensor data
 ↓
Time-series parser
```

The output is not treated as final truth.

Instead, the extraction layer generates **candidate observations** that are converted into structured claims.

---

## 5. Unified Evidence Representation

The extracted information is represented using a common evidence model.

The central object is the `ClaimRecord`.

Conceptually:

```text
ClaimRecord
│
├── Subject
├── Predicate
├── Object / Value
├── Temporal Context
├── Provenance
├── Claim Type
├── Confidence
└── Status
```

Example:

```text
Claim ID:
CLAIM-1042

Subject:
Pump-B

Predicate:
temperature

Value:
94

Unit:
°C

Time:
09:21 UTC

Type:
Extracted

Source:
maintenance_report.pdf
Page 7

Confidence:
0.94
```

This common representation allows information from different modalities to be processed using the same downstream reasoning framework.

---

## 6. Provenance Tracking

Every claim maintains a relationship with its originating evidence.

AERIS uses modality-specific evidence pointers.

### Text

```text
Document
 └── Page
      └── Text range
```

### Image

```text
Image
 └── Bounding region
```

### Table

```text
Table
 └── Row
      └── Column
```

### Sensor

```text
Time-Series
 └── Timestamp
```

This allows the system to answer:

> "Why does AERIS believe this claim?"

by directly identifying the supporting evidence.

---

## 7. Entity Resolution Layer

The entity-resolution layer connects references across sources.

For example:

```text
Source A → Pump-B

Source B → Pump B

Source C → P-002

Source D → Pump_02
```

AERIS evaluates whether these references correspond to the same canonical entity.

The result may be:

```text
Pump-B
 ├── Pump B
 ├── P-002
 └── Pump_02
```

The system must also represent unresolved relationships.

For example:

```text
CV-E
CV-E2
```

may produce:

```text
AMBIGUOUS
```

rather than an automatic merge.

---

## 8. Temporal Alignment Layer

Once entities have been resolved, AERIS determines when claims apply.

The temporal layer processes:

* Exact timestamps
* Time intervals
* Approximate timestamps
* Time zones
* Observation windows
* Event ordering

Example:

```text
09:20       09:21       09:22       09:23
              │           │
              ▼           ▼
           Sensor       Report
           72.3°C        94°C
              └─────┬─────┘
                overlap
```

The overlap information becomes an input to the conflict engine.

---

## 9. Normalization Layer

Different sources may represent the same information differently.

AERIS normalizes comparable representations before reasoning.

Examples include:

### Units

```text
158°F
 ↓
≈ 70°C
```

### Names

```text
Pump B
Pump-B
P-002
 ↓
Canonical entity
```

### Time

```text
Local time
 ↓
UTC
```

Normalization ensures that superficial differences do not become false conflicts.

---

## 10. Evidence Reasoning Layer

The evidence reasoning layer compares claims after entity resolution, temporal alignment, and normalization.

A simplified reasoning sequence is:

```text
Are the entities the same?
          │
          ▼
Are the attributes the same?
          │
          ▼
Do the temporal contexts overlap?
          │
          ▼
Are the values comparable?
          │
          ▼
Is the difference significant?
          │
          ▼
     Classification
```

Possible classifications include:

```text
AGREEMENT
CONFLICT
AMBIGUOUS
INSUFFICIENT EVIDENCE
NOT COMPARABLE
```

---

## 11. Conflict Preservation

AERIS does not replace conflicting claims with an averaged or selected value.

For example:

```text
Sensor
72.3°C
      │
      ├──────────┐
      │          │
      ▼          ▼
             CONFLICT
      ▲          │
      │          │
Report          │
94°C ───────────┘
```

Both claims remain available.

The conflict itself becomes a structured relationship between the claims.

This allows users to inspect the evidence that caused the conflict.

---

## 12. Confidence and Uncertainty

AERIS maintains confidence information throughout the processing pipeline.

Confidence may be associated with:

```text
Extraction
Entity Resolution
Temporal Alignment
Source Reliability
Inference
```

Rather than treating confidence as a single opaque value, the system maintains its contributing dimensions.

This allows users to distinguish:

```text
High extraction confidence
+
Low entity confidence
```

from:

```text
High extraction confidence
+
High entity confidence
+
High temporal confidence
```

The system can therefore communicate not only a result but also the uncertainty surrounding that result.

---

## 13. Investigation Layer

The final layer provides a human-readable representation of the evidence state.

The interface should allow users to inspect:

### Case

```text
CASE-047
```

### Entities

```text
Pump-B
Motor-C
Compressor-D
```

### Events

```text
Inspection
Temperature Alarm
Maintenance
```

### Claims

```text
Temperature = 72.3°C
```

### Conflicts

```text
72.3°C vs 94°C
```

### Evidence

```text
sensor.csv → Row 481

maintenance_report.pdf → Page 7
```

The user should be able to navigate from the conclusion back to the original evidence.

---

## 14. End-to-End Example

Consider the following sources:

```text
maintenance_report.pdf
sensor_temperature.csv
inspection.png
technician_notes.txt
```

AERIS processes them:

```text
                 SOURCES
                    │
                    ▼
              EXTRACTION
                    │
                    ▼
              CLAIM RECORDS
                    │
                    ▼
             ENTITY LINKING
                    │
                    ▼
            TEMPORAL ALIGNMENT
                    │
                    ▼
             NORMALIZATION
                    │
                    ▼
          EVIDENCE COMPARISON
                    │
             ┌──────┼──────┐
             ▼      ▼      ▼
          AGREE  CONFLICT  AMBIGUOUS
                    │
                    ▼
              CONFIDENCE
                    │
                    ▼
              INVESTIGATION
```

For example:

```text
Sensor:
Pump-B = 72.3°C
09:21:43 UTC

Report:
Pump-B = 94°C
09:22 UTC
```

AERIS determines:

```text
Same entity          ✓
Same attribute       ✓
Temporal overlap     ✓
Comparable units     ✓
Difference > tolerance ✓

RESULT:
GENUINE CONFLICT
```

The interface then provides the supporting evidence for both claims.

---

## 15. System Characteristics

The proposed architecture is designed to provide:

### Traceability

Every claim can be traced to source evidence.

### Modularity

Extraction, entity resolution, temporal reasoning, and conflict detection are independent components.

### Deterministic Validation

Critical evidence relationships can be evaluated using explicit rules.

### Uncertainty Awareness

Ambiguous and incomplete information is represented explicitly.

### Multimodal Support

Different evidence types can participate in the same reasoning process.

### Extensibility

New extraction providers and source formats can be added without redesigning the core evidence model.

---

## 16. Conceptual Architecture

The complete AERIS concept can therefore be summarized as:

```text
                    ┌─────────────────────┐
                    │   MULTIMODAL INPUT  │
                    │                     │
                    │ Text | Image | Data │
                    └──────────┬──────────┘
                               │
                               ▼
                    ┌─────────────────────┐
                    │      INGESTION      │
                    └──────────┬──────────┘
                               │
                               ▼
                    ┌─────────────────────┐
                    │     EXTRACTION      │
                    │                     │
                    │ AI / OCR / Parsers  │
                    └──────────┬──────────┘
                               │
                               ▼
                    ┌─────────────────────┐
                    │  EVIDENCE MODEL     │
                    │                     │
                    │ Claims / Entities   │
                    │ Events / Provenance │
                    └──────────┬──────────┘
                               │
                 ┌─────────────┼─────────────┐
                 ▼             ▼             ▼
          ENTITY RESOLUTION  TEMPORAL   NORMALIZATION
                            ALIGNMENT
                 │             │             │
                 └─────────────┼─────────────┘
                               ▼
                    ┌─────────────────────┐
                    │ EVIDENCE REASONING  │
                    └──────────┬──────────┘
                               │
                ┌──────────────┼──────────────┐
                ▼              ▼              ▼
            AGREEMENT       CONFLICT       AMBIGUITY
                               │
                               ▼
                    ┌─────────────────────┐
                    │ CONFIDENCE / AUDIT  │
                    └──────────┬──────────┘
                               │
                               ▼
                    ┌─────────────────────┐
                    │ INVESTIGATION UI    │
                    └─────────────────────┘
```

---

## 17. System Concept Summary

AERIS transforms heterogeneous evidence into a common, provenance-aware representation and progressively establishes relationships between observations.

The system does not attempt to hide disagreement or force incomplete evidence into definitive conclusions. Instead, it preserves the original observations and makes their relationships explicit.

The fundamental processing principle is:

> **Extract → Structure → Link → Align → Normalize → Reason → Explain.**

---

**Next in the docs sequence: `03-requirements.md`.** That document will convert this concept into concrete **functional requirements, non-functional requirements, system constraints, inputs/outputs, and acceptance criteria**.
