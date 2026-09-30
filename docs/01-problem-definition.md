# AERIS Problem Definition

## 1. Problem Statement

Modern operational and intelligent systems generate information through multiple heterogeneous sources, including textual reports, images, structured tables, event logs, and time-series sensor data.

Although these sources may describe the same real-world entities and events, they often differ in:

* Naming conventions
* Data formats
* Measurement units
* Timestamp formats
* Temporal granularity
* Level of detail
* Reliability
* Completeness
* Degree of uncertainty

As a result, combining information from these sources is not simply a matter of retrieving semantically similar content.

A system must determine whether observations from different sources refer to the same entity, whether they describe the same event or measurement, whether their temporal contexts overlap, and whether their values are actually comparable.

The central problem addressed by AERIS is therefore:

> **How can heterogeneous multimodal evidence be transformed into a unified and inspectable representation while preserving provenance, temporal context, uncertainty, entity identity, and contradictions between sources?**

---

## 2. Existing Gap

Conventional information-retrieval and multimodal AI systems are effective at finding related information, generating summaries, and answering questions across documents.

However, semantic similarity alone does not establish that two pieces of evidence:

1. Refer to the same real-world entity
2. Describe the same attribute or event
3. Apply to the same point or interval in time
4. Use comparable measurements
5. Actually contradict one another

For example, two sources may contain:

```text
Source A:
Pump-B temperature = 72 °C

Source B:
Pump-B temperature = 158 °F
```

A numerical or textual comparison may treat these as different values, even though they represent approximately the same temperature after unit normalization.

Conversely:

```text
Source A:
Pump-B temperature = 72 °C

Source B:
Pump-B temperature = 94 °C
```

may represent a genuine conflict when the observations refer to the same entity, attribute, and overlapping time period.

AERIS addresses this gap by introducing an explicit evidence representation and reasoning layer between multimodal extraction and final interpretation.

---

## 3. Core Research / Engineering Question

AERIS focuses on the following engineering question:

> **Can heterogeneous multimodal observations be converted into structured claims and reliably reconciled using provenance-aware entity resolution, temporal alignment, measurement normalization, and deterministic conflict reasoning?**

The system must preserve the underlying observations rather than collapsing them into a single generated answer.

---

## 4. Input Scope

AERIS is designed to process multiple evidence modalities.

### 4.1 Text

Examples:

* Maintenance reports
* Technician notes
* Operational logs
* Incident descriptions
* Structured textual observations

### 4.2 Images

Examples:

* Inspection photographs
* Equipment images
* Visual observations
* Annotated images

### 4.3 Structured Data

Examples:

* CSV files
* Tables
* JSON records
* Maintenance databases
* Component registries

### 4.4 Time-Series Data

Examples:

* Temperature telemetry
* Vibration measurements
* Pressure readings
* Machine state signals
* Event streams

---

## 5. Expected Output

AERIS converts incoming evidence into a unified representation containing:

### Entities

Objects or real-world subjects identified across sources.

```text
Pump-B
Motor-C
Compressor-D
```

### Events

Occurrences associated with entities and time.

```text
Inspection
Maintenance
Temperature Alarm
Component Failure
```

### Claims

Structured observations or derived statements.

```text
Pump-B temperature = 94 °C
```

### Temporal Context

Information describing when a claim or event applies.

```text
09:21:43 UTC
```

or:

```text
09:20:00 → 09:23:00 UTC
```

### Provenance

A pointer to the exact supporting source.

```text
sensor.csv
Row 481
Timestamp 09:21:43
```

### Confidence

Information describing the reliability of extraction, resolution, alignment, or inference.

---

## 6. Required Reasoning

AERIS must perform reasoning across multiple dimensions.

### 6.1 Entity Resolution

Determine whether differently named references correspond to the same entity.

Example:

```text
Pump-B
Pump B
P-002
Pump_02
```

Potentially:

```text
        ┌── Pump-B
        ├── Pump B
PUMP ───┼── P-002
        └── Pump_02
```

When evidence is insufficient, the system must preserve the ambiguity instead of forcing an identity.

---

### 6.2 Temporal Alignment

Determine whether claims refer to compatible time periods.

The system must support:

* Point timestamps
* Time intervals
* Time zones
* Approximate times
* Temporal overlap
* Event ordering

This prevents unrelated observations from being classified as conflicts.

---

### 6.3 Measurement Normalization

Measurements must be converted into comparable representations where possible.

Example:

```text
158 °F
      ↓
≈ 70 °C
```

The system must distinguish a unit difference from an actual disagreement.

---

### 6.4 Conflict Detection

AERIS should identify a conflict only when sufficient conditions are satisfied.

Conceptually:

```text
Same Entity
     +
Same Attribute
     +
Temporal Compatibility
     +
Comparable Representation
     +
Value Difference Beyond Tolerance
     =
Potential Genuine Conflict
```

The system must retain both claims involved in the conflict.

---

## 7. Evidence Classification

AERIS should not force every relationship into a binary true/false classification.

The target classifications are:

| Status                    | Meaning                                                                  |
| ------------------------- | ------------------------------------------------------------------------ |
| **Agreement**             | Evidence is compatible after appropriate normalization                   |
| **Conflict**              | Comparable evidence contradicts another claim                            |
| **Ambiguous**             | Available evidence is insufficient to establish identity or relationship |
| **Insufficient Evidence** | Required evidence is missing                                             |
| **Not Comparable**        | Claims cannot meaningfully be compared                                   |

This classification allows the system to represent uncertainty explicitly.

---

## 8. Provenance Requirement

A core requirement is that generated claims remain connected to their source evidence.

For example:

```text
Claim
  │
  └── Temperature = 94 °C
          │
          └── maintenance_report.pdf
                  │
                  └── Page 7
```

For other modalities:

```text
Image
 └── Bounding region

CSV
 └── Row / Column

Sensor
 └── Timestamp
```

A claim without traceable provenance should not be treated as fully inspectable evidence.

---

## 9. Extracted vs. Inferred Information

AERIS distinguishes between information directly present in a source and information derived through reasoning.

### Extracted

```text
Sensor reports:
94 °C
```

### Inferred

```text
A temperature anomaly occurred.
```

This distinction prevents inferred conclusions from being presented as though they were directly observed.

---

## 10. Missing and Ambiguous Data

AERIS must remain functional when evidence is incomplete.

### Missing Data

If telemetry is unavailable:

```text
09:25 → 09:29

No sensor data
```

The system should represent:

```text
INSUFFICIENT EVIDENCE
```

rather than inventing a measurement or assuming normal operation.

### Ambiguous Data

If two entity references cannot be reliably resolved:

```text
CV-E
CV-E2
```

the system should preserve:

```text
AMBIGUOUS ENTITY
```

rather than silently merging them.

---

## 11. Non-Goals

AERIS is not intended to:

* Replace human judgment in high-impact decisions
* Automatically declare an absolute source of truth
* Remove contradictory evidence
* Average conflicting observations into a single value
* Treat semantic similarity as proof of identity
* Use an LLM as the sole conflict-detection mechanism
* Hide uncertainty behind a single confidence score

The system is intended to make evidence relationships **explicit and inspectable**.

---

## 12. Success Criteria

A successful AERIS implementation should demonstrate that it can:

1. Process text, images, structured data, and time-series evidence.
2. Convert observations into structured claims.
3. Preserve claim-level provenance.
4. Resolve equivalent entity references across sources.
5. Preserve ambiguity when entity identity cannot be established.
6. Align observations according to their temporal context.
7. Normalize comparable measurements and units.
8. Detect genuine conflicts without suppressing the underlying evidence.
9. Distinguish agreement, conflict, ambiguity, insufficient evidence, and non-comparability.
10. Separate extracted observations from inferred conclusions.
11. Represent confidence and uncertainty explicitly.
12. Evaluate its reasoning against known ground truth.

---

## 13. Problem Boundary

The primary engineering focus of AERIS is **evidence reconciliation**.

The system therefore concentrates on the transformation:

```text
Heterogeneous Evidence
        ↓
Structured Claims
        ↓
Aligned Evidence
        ↓
Reasoned Relationships
        ↓
Inspectable Results
```

The objective is not to build a general-purpose autonomous AI system.

The objective is to provide a reliable evidence layer that allows multimodal AI systems and human investigators to understand how information from different sources relates, agrees, disagrees, or remains uncertain.

---

**Next in the docs sequence: `02-system-concept.md`.** That document will convert this problem definition into the actual AERIS solution concept, including the major components and end-to-end processing flow.
