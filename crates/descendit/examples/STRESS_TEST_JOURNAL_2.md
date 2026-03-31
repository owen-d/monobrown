# Stress Test Journal 2: AI Agent Refactoring Loop with `descendit guide`

**Date:** 2026-03-30
**Operator:** AI agent (Claude), following only what `descendit guide` provides
**Target codebase:** `crates/descendit/examples/sample-crate/src/` (5 files, ~521 lines)
**Constraint:** `--semantic off` (standalone crate, not in workspace)

---

## 1. Learning Phase: Running the Guide

### Command

```
cargo run -q -- guide
```

### What the Guide Told Me

The guide is well-structured. It covers:
- Quick start command: `descendit analyze src/ --agent --top 5`
- Core workflow diagram: `analyze --> diff --> comply` with `heatmap` as a drill-down
- All subcommands with flags and examples
- Loss dimensions (duplication, state_cardinality, bloat, code_economy, coupling_density)
- Composite loss formula: `1 - geometric_mean(dimension_scores)`
- The "gradient-descent refactoring workflow" (5 steps: baseline, identify hotspots, refactor, re-score, converge)
- Convergence criterion: stop when composite loss delta < 0.005

### Assessment of the Guide

**Strengths:**
- The workflow section is clear and actionable: baseline -> heatmap -> refactor -> diff -> repeat.
- The flag tables are complete enough to get started.
- The composite loss formula explanation is helpful for interpreting results.
- The convergence threshold (0.005) gives a concrete stopping criterion.

**Weaknesses and Gaps (detailed below in Section 5).**

---

## 2. Epoch 0: Baseline Measurement

### Commands Run

```
cargo run -q -- analyze crates/descendit/examples/sample-crate/src/ --semantic off > baseline_epoch0.json
cargo run -q -- analyze crates/descendit/examples/sample-crate/src/ --semantic off --agent --top 10
cargo run -q -- heatmap crates/descendit/examples/sample-crate/src/ --semantic off --tree
cargo run -q -- analyze crates/descendit/examples/sample-crate/src/ --semantic off --compliance
```

### Baseline Results

| Metric | Value |
|--------|-------|
| Composite loss | **0.1285** |
| state_cardinality loss | 0.3835 |
| bloat loss | 0.1846 |
| duplication loss | 0.0 |
| code_economy loss | 0.0 |
| coupling_density loss | 0.0 (not measured) |

### Top Heatmap Items (by responsibility)

| Item | Dimension | Responsibility | Detail |
|------|-----------|---------------|--------|
| ProcessingFeatures | state_cardinality | 0.0159 | log2 cardinality 5.0 |
| OutputConfig | state_cardinality | 0.0159 | log2 cardinality 5.0 |
| RunMode | state_cardinality | 0.0101 | log2 cardinality 4.0 |
| IncludeFields | state_cardinality | 0.0101 | log2 cardinality 4.0 |
| ContentChecks | state_cardinality | 0.0101 | log2 cardinality 4.0 |
| ingest_records | bloat | 0.0060 | 44 lines |

### Observations

- `state_cardinality` dominates, accounting for ~84.5 total responsibility across 9 types vs ~35.6 for bloat across 20 functions.
- The two worst offenders both have log2 cardinality = 5.0 (i.e., 32 states from 5 boolean fields).
- Bloat is secondary; `ingest_records` at 44 lines is the top bloat contributor but far below the 70-line threshold mentioned in the raw analysis summary.
- Duplication and code_economy are already at 0.0 loss -- nothing to fix there.

---

## 3. Epoch 1: Reducing Top State Cardinality

### Strategy

Target the two structs with log2=5.0 (the maximum offenders):

1. **ProcessingFeatures** (5 bools) -- split into `EncodingFeatures` (enable_compression, enable_encryption) and `IntegrityFeatures` (skip_validation, enable_dedup, retry_on_failure). The parent struct becomes a composition with 0 direct bool fields.

2. **OutputConfig** (5 direct bools + include sub-struct) -- extract `sort_output` and `deduplicate` into a `PostProcessing` sub-struct. Reduces direct bool count from 5 to 3.

### Changes Made

**config.rs:**
- Split `ProcessingFeatures` (5 bools, log2=5.0) into `EncodingFeatures` (2 bools, log2=2.0) + `IntegrityFeatures` (3 bools, log2=3.0), composed in `ProcessingFeatures` (0 direct bools, log2=0.0).
- Split `OutputConfig` (5 direct bools) by extracting `PostProcessing { sort_output, deduplicate }` (2 bools, log2=2.0), leaving `OutputConfig` with 3 direct bools (log2=3.0).

**ingest.rs:**
- Updated field access paths: `config.processing.enable_dedup` -> `config.processing.integrity.enable_dedup`

**output.rs:**
- Updated field access paths: `config.sort_output` -> `config.post_processing.sort_output`, etc.

### Verification

```
cd crates/descendit/examples/sample-crate && cargo check
# Result: compiles successfully
```

### Measurement Commands

```
cargo run -q -- analyze crates/descendit/examples/sample-crate/src/ --semantic off > epoch1.json
cargo run -q -- diff baseline_epoch0.json epoch1.json --compliance
cargo run -q -- analyze crates/descendit/examples/sample-crate/src/ --semantic off --agent --top 10
```

### Epoch 1 Results

| Metric | Before | After | Delta |
|--------|--------|-------|-------|
| Composite loss | 0.1285 | **0.0989** | **-0.0296** |
| state_cardinality | 0.3835 | 0.2722 | -0.1114 |
| bloat | 0.1846 | 0.1838 | -0.0008 |

### Error Encountered

When running `diff --compliance`, I initially tried:
```
cargo run -q -- diff baseline.json epoch1.json --compliance --semantic off
```
This **failed** with:
```
error: unexpected argument '--semantic' found
  tip: a similar argument exists: '--semantic-path'
```

The `diff` subcommand does not accept `--semantic off`. It only accepts `--semantic-path`.
Dropping `--semantic off` worked fine because `diff` operates on pre-computed JSON snapshots that were already generated with `--semantic off`.

**This is a gap in the guide.** The guide shows `--semantic` as a flag for `analyze`, `comply`, and `heatmap`, but does not mention it for `diff`. However, the guide also does not explicitly say that `diff` does NOT need it. An agent has to infer this, or learn by error.

---

## 4. Epoch 2: Reducing Remaining State Cardinality + Bloat

### Strategy

Target the next tier of offenders:

1. **RunMode** (4 bools, log2=4.0) -- split into `RuntimeBehavior` (verbose, parallel) and `SafetyConstraints` (dry_run, strict_mode).
2. **IncludeFields** (4 bools, log2=4.0) -- split into `StructuralIncludes` (headers, line_numbers) and `MetadataIncludes` (timestamps, source).
3. **ContentChecks** (4 bools, log2=4.0) -- split into `TextChecks` (utf8, empty_fields) and `ValueChecks` (duplicates, numeric_ranges).
4. **ingest_records** (44 lines, top bloat) -- extract `dedup_records()` helper and `parse_all_lines()` helper to reduce the main function from 44 lines to 21 lines.

### Changes Made

**config.rs:**
- `RunMode` split into `RuntimeBehavior` + `SafetyConstraints` (each 2 bools, log2=2.0).
- `IncludeFields` split into `StructuralIncludes` + `MetadataIncludes` (each 2 bools, log2=2.0).
- `ContentChecks` split into `TextChecks` + `ValueChecks` (each 2 bools, log2=2.0).
- Updated all Default impls for the new types.

**ingest.rs:**
- Extracted `pub fn dedup_records(records: &mut Vec<Record>) -> usize` (5 lines).
- Extracted `pub fn parse_all_lines(...)` (15 lines) -- contains the line-parsing loop with error handling.
- `ingest_records` reduced from 44 lines to 21 lines.

**output.rs, ingest.rs:**
- Updated all field access paths (e.g., `run_mode.verbose` -> `run_mode.behavior.verbose`, `include.headers` -> `include.structural.headers`, `content.utf8` -> `content.text.utf8`, etc.)

### Verification

```
cd crates/descendit/examples/sample-crate && cargo check
# Result: compiles successfully
```

### Measurement Commands

```
cargo run -q -- analyze crates/descendit/examples/sample-crate/src/ --semantic off > epoch2.json
cargo run -q -- diff epoch1.json epoch2.json --compliance
cargo run -q -- diff baseline_epoch0.json epoch2.json --compliance
cargo run -q -- analyze crates/descendit/examples/sample-crate/src/ --semantic off --agent --top 10
cargo run -q -- heatmap crates/descendit/examples/sample-crate/src/ --semantic off --tree
cargo run -q -- diff baseline_epoch0.json epoch2.json --heatmap
```

### Epoch 2 Results (vs Epoch 1)

| Metric | Epoch 1 | Epoch 2 | Delta |
|--------|---------|---------|-------|
| Composite loss | 0.0989 | **0.0653** | **-0.0336** |
| state_cardinality | 0.2722 | 0.1626 | -0.1096 |
| bloat | 0.1838 | 0.1480 | -0.0357 |

### Cumulative Results (Epoch 0 -> Epoch 2)

| Metric | Epoch 0 | Epoch 2 | Delta | % Improvement |
|--------|---------|---------|-------|---------------|
| Composite loss | 0.1285 | **0.0653** | **-0.0632** | 49.2% |
| state_cardinality | 0.3835 | 0.1626 | -0.2210 | 57.6% |
| bloat | 0.1846 | 0.1480 | -0.0365 | 19.8% |
| duplication | 0.0 | 0.0 | 0.0 | -- |
| code_economy | 0.0 | 0.0 | 0.0 | -- |

### Heatmap Diff Summary (baseline -> epoch 2)

- **11 items appeared** (the new sub-structs and extracted functions)
- **4 items disappeared** (the original high-cardinality structs: ProcessingFeatures, RunMode, IncludeFields, ContentChecks)
- **22 items improved** (all existing items got lower responsibility scores)
- **0 items regressed**

Notable changes:
- OutputConfig: 0.0159 -> 0.0035 (delta -0.0124)
- ingest_records: 0.0060 -> 0.0026 (delta -0.0034)
- All the log2=4.0 state_cardinality items disappeared entirely

### Convergence Check

The guide says to stop when composite loss delta < 0.005.
- Epoch 1 delta: -0.0296 (continue)
- Epoch 2 delta: -0.0336 (continue -- but we've completed our 2 required epochs)

The codebase could benefit from further epochs, but diminishing returns are setting in since remaining items are all log2=2.0 or log2=3.0 for state_cardinality, and no function exceeds 33 lines for bloat.

---

## 5. Guide Quality Assessment

### Was the Guide Sufficient to Complete the Task?

**Mostly yes.** The guide provided enough information to:
- Take a baseline with `analyze ... > baseline.json`
- Identify hotspots with `heatmap --tree` and `--agent --top N`
- Make targeted refactoring decisions based on the heatmap output
- Measure improvement with `diff baseline.json current.json --compliance`
- Understand when to stop (delta < 0.005)

### What Worked Well

1. **The workflow section is excellent.** Steps 1-5 map directly to what an agent needs to do. The numbered steps with concrete commands are easy to follow.
2. **`--agent` flag is very useful.** The compact JSON with top heatmap items, dimension totals, and per-dimension losses is exactly what a programmatic consumer needs.
3. **`diff --compliance`** output is clean and actionable -- shows before/after/delta/assessment per dimension.
4. **`diff --heatmap`** output is outstanding -- appeared/disappeared/changed categories with deltas make it trivial to verify that a refactoring had the intended effect.
5. **The loss dimensions table** gives enough context to understand what each dimension measures.
6. **The convergence criterion** (delta < 0.005) is concrete and measurable.

### Pain Points and Issues

1. **`diff` does not accept `--semantic off`**, but the guide doesn't clarify this. I had to learn by trial and error. The guide's flag table for `diff` correctly omits `--semantic`, but an agent following the "gradient-descent workflow" might not notice this omission. Since the workflow section shows `descendit analyze src/ > baseline.json` and later `descendit diff baseline.json current.json --compliance`, the user has to independently realize that `--semantic off` (used during analyze) is not needed during diff. A one-line note in the workflow section would help: "Note: `diff` operates on pre-computed snapshots, so `--semantic` is not needed."

2. **No guidance on what state_cardinality *specifically* measures or how to reduce it.** The dimension table says "State-space size of types (enum variants, struct fields, booleans)" but doesn't explain the scoring. I had to infer from the heatmap output (which shows "log2 cardinality 5.0") that it's counting boolean fields and scoring based on the log2 of the combinatorial state space (2^n for n bools). The guide could add a sentence: "For structs, state cardinality is 2^(bool_fields) * 2^(option_fields). Reduce it by splitting bool-heavy types into focused sub-structs."

3. **No guidance on what bloat *specifically* measures or how to reduce it.** The table says "Function length and complexity beyond thresholds" but doesn't say what the threshold is or what the scoring curve looks like. From the raw analysis JSON I could see `lines`, `cyclomatic`, and `nesting_depth` are tracked, but the heatmap only showed "44 lines" -- it wasn't clear whether cyclomatic complexity or nesting depth also contribute to the bloat score. A brief note on the scoring factors would help.

4. **`--compliance` output (from `analyze`) is extremely verbose.** The full compliance JSON is ~31KB for a 521-line codebase. It dumps the entire normalization/calibration pipeline for each dimension. The `--agent` flag is far more useful for programmatic workflows. The guide could note: "For agent workflows, prefer `--agent` over `--compliance` for concise output."

5. **The guide's workflow section uses `src/` as the path**, but when running from a workspace root, the actual path needs to be `crates/descendit/examples/sample-crate/src/`. This is not a guide deficiency per se, but the quick-start example could note that the path should point to the crate root or source directory being analyzed.

6. **No mention of what happens when `--semantic off` is used with `comply`.** The guide shows `descendit comply analysis.json --semantic off` as a valid example, so this is covered -- but it's worth noting that `coupling_density` will show `loss: 0.0` with `not_measured: true` in this mode, which affects the composite loss calculation. The guide doesn't explain how unmeasured dimensions affect the geometric mean.

7. **The heatmap `--tree` flag is underdocumented.** The guide shows it produces a "hierarchical rollup tree" but doesn't explain the format. I could infer from the output that indented items (prefixed with `` `-- ``) are nested within a parent type, and the number in parentheses is total responsibility. But a brief format description would help.

8. **No explicit mention that `analyze` output is the snapshot format for `diff`.** The guide shows `descendit analyze src/ > baseline.json` and then `descendit diff baseline.json current.json`, so it's implied -- but it would be clearer to state: "The JSON output of `analyze` (without `--compliance`, `--agent`, or `--loss-vector`) is the snapshot format expected by `diff` and `comply`."

### What I Had to Figure Out on My Own

1. That `diff` doesn't accept `--semantic` -- learned by error.
2. That state_cardinality counts `2^(bool_fields)` -- inferred from heatmap "log2 cardinality" detail strings and the raw analysis JSON's `state_cardinality_log2` field.
3. That the raw `analyze` JSON (not `--agent` or `--compliance`) is the correct input format for `diff` and `comply`.
4. That `--agent` output includes both per-dimension losses AND top heatmap items, making it more useful than running `analyze` + `heatmap` separately for a quick overview.
5. That bloat scoring is primarily driven by line count (the heatmap detail strings only mention lines, not cyclomatic complexity).
6. That splitting a struct's boolean fields into sub-structs effectively zeroes out the parent's direct bool count, eliminating its state_cardinality contribution while adding smaller contributions from the sub-structs.

### Suggested Guide Improvements

1. Add a note to the workflow section: "`diff` operates on pre-computed snapshots; `--semantic` is not needed."
2. Add brief scoring explanations for each dimension (e.g., "state_cardinality: 2^(bool_fields + option_fields); reduce by splitting bool-heavy types").
3. Clarify that raw `analyze` output (no extra flags) is the snapshot format for `diff`/`comply`.
4. Add a note about `--agent` being preferred for programmatic/agent workflows over `--compliance`.
5. Explain how unmeasured dimensions (e.g., coupling_density with `--semantic off`) affect the composite loss.
6. Briefly describe the heatmap tree format (parent items, nested children, responsibility scores).

---

## 6. Summary

The `descendit guide` is functional and provides enough information for an AI agent to complete a full refactoring loop. Over 2 epochs, the composite loss was reduced by **49.2%** (0.1285 -> 0.0653). The primary improvement vector was state_cardinality (57.6% reduction), achieved by splitting boolean-heavy structs into focused sub-structs. A secondary improvement in bloat (19.8% reduction) came from extracting helper functions.

The guide's main gap is in explaining the *scoring mechanics* of each dimension. An agent can work around this by examining heatmap detail strings and raw analysis JSON, but explicit documentation would reduce trial-and-error and make the tool more accessible.

The tool itself is well-designed for agent workflows: the `--agent` flag, structured JSON output, and `diff --heatmap` output are particularly well-suited to programmatic consumption.
