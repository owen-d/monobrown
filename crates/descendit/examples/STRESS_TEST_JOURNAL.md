# Stress Test Journal: First-Time User Experience with descendit

**Date:** 2026-03-30
**Tester:** Claude Code agent (simulating a first-time user)
**Target:** `crates/descendit/examples/sample-crate/src/` (5 files, ~500 lines)
**Workflow:** 4 epochs of analyze -> heatmap -> fix -> diff

---

## Summary of the Refactoring Loop

| Epoch | Composite Loss | Delta   | Action                                                    |
|-------|---------------|---------|-----------------------------------------------------------|
| 0     | 0.590         | --      | baseline                                                  |
| 1     | 0.213         | -0.377  | split bool-soup structs, remove unused private helpers     |
| 2     | 0.172         | -0.040  | deduplicate CSV/TSV transform, split ValidationChecks      |
| 3     | 0.145         | -0.027  | make extracted helpers pub to improve code_economy          |

**Total improvement:** 0.590 -> 0.145 (delta -0.445, 75% reduction)

Per-dimension journey (epoch 0 -> 3):

| Dimension          | Epoch 0 | Epoch 3 | Delta   |
|--------------------|---------|---------|---------|
| state_cardinality  | 0.970   | 0.384   | -0.587  |
| code_economy       | 0.473   | 0.092   | -0.381  |
| duplication        | 0.097   | 0.000   | -0.097  |
| bloat              | 0.176   | 0.185   | +0.009  |
| coupling_density   | 0.000   | 0.000   | 0.000   |

---

## What Worked Well

1. **The heatmap is the killer feature.** It immediately tells you what to fix and
   why. The ranked list with dimension attribution made it obvious that
   `state_cardinality` in `config.rs` was the dominant problem. Without this, I
   would have guessed "bloat" was the main issue (the big functions look scarier
   than the boolean soup).

2. **`--agent` mode is perfect for LLM consumption.** The compact JSON with
   `composite_loss`, per-dimension breakdowns, and `top_heatmap` gave me
   everything needed to plan a fix in one call. The `responsibility` field in
   heatmap entries is particularly useful for prioritization.

3. **The diff command closes the feedback loop cleanly.** Being able to
   `diff epoch0.json epoch1.json --compliance` and see per-dimension
   assessment ("improved" / "regressed" / "unchanged") is exactly what you need
   for iterative refactoring. The convergence pattern from the README (stop when
   |delta| < 0.005) is practical and matches what I observed.

4. **`descendit list` is excellent.** The detailed descriptions of each dimension
   with formulas and aggregation strategies answered questions before I had to
   ask them. This is a model reference command.

5. **`descendit guide` is comprehensive.** It contains everything a first-time
   user needs. Having it built into the binary (instead of only in the README)
   means you always have the reference available.

6. **Error messages are clear.** Running on a nonexistent path gives
   `no .rs files found in nonexistent_path/` -- concise and actionable.

7. **`--semantic off` works reliably.** Crucial for quick iteration. The tool
   is fast (sub-second) in syntactic-only mode.

---

## What Was Confusing or Frustrating

### 1. The snapshot/diff workflow is fragile with `cargo run`

**Pain:** When saving a snapshot via `cargo run -- analyze ... > epoch0.json`,
Cargo's build output ("Finished...", "Running...") is written to stdout and
pollutes the JSON file. This causes `diff` to fail with a cryptic error:

```
Error: expected value at line 1 column 5
```

**Root cause:** Cargo's own status messages go to stdout, not stderr. The user
must use `cargo run -q` (quiet) or redirect stderr, which is not mentioned in
the README.

**Suggestion:** The README examples should use `cargo run -q --` or note this
caveat. Even better: `descendit analyze` could detect that stdout is being piped
to a file and emit a warning if non-JSON content would be written. Or the diff
command could skip leading non-JSON lines gracefully.

### 2. No guidance on what score improvements are "good enough"

The README shows a convergence table (stop at delta < 0.005) but doesn't say
what absolute loss values are acceptable. After epoch 3, my composite loss is
0.145 -- is that good? The heatmap shows all entries below 0.016 responsibility.
There is no way to tell if further improvement is worth the effort without a
reference frame.

**Suggestion:** Add a qualitative scale, e.g.:
- 0.00 - 0.05: excellent
- 0.05 - 0.15: good
- 0.15 - 0.30: room for improvement
- 0.30+: significant quality issues

### 3. `--agent` vs `--compliance` vs bare `analyze` is unclear

The README mentions `--agent` for "composite loss + hotspots" and `--compliance`
appears in diff but is also available on analyze. As a new user, I was confused
about which output format to use when. The bare `analyze` dumps the full raw
metrics JSON (hundreds of lines), which is not useful for interactive use.

**Suggestion:** The README quick start should explain the three modes more
clearly:
- `--agent`: compact summary for LLM/CI consumption
- `--compliance`: detailed scoring report with policy rules
- (bare): raw data for tooling/scripting

### 4. Heatmap `responsibility` values are hard to interpret

The heatmap shows values like `0.196` for `PipelineConfig` state_cardinality.
It is not clear what this number means in isolation. Is it a fraction of total
loss? A per-item score? The `detail` field ("log2 cardinality 15.0") helps, but
the `responsibility` column lacks context.

**Suggestion:** Either document what responsibility means (e.g., "fraction of
total composite loss attributable to this item") or show it as a percentage.

### 5. The `--semantic off` flag name is unintuitive

The flag takes `require` or `off` as values. The name `--semantic` suggests a
boolean toggle, but `--semantic false` or `--semantic no` don't work (only
`require` and `off`). A more standard CLI pattern would be `--no-semantic` for
the off case, or `--semantic-mode off|require`.

Also, the default behavior was surprising: running without `--semantic off` on
a crate outside a workspace silently succeeded and produced semantic data with
`coupling_density: 0.5`. I expected it to either fail (since there's no
Cargo.toml workspace for RA to analyze) or produce no coupling data.

### 6. `code_economy` penalizes good design

In epoch 1, I extracted several private helper functions from a bloated public
function (good refactoring), but this increased the `code_economy` loss because
the private/public ratio went up. The "right" fix from descendit's perspective
was to make them `pub` -- but that's not always the right design choice. Making
internal helpers public just to satisfy a metric is backwards.

**Suggestion:** Consider exempting functions that are called by public functions
in the same module, or provide a way to annotate "intentional private helper"
in the policy.

### 7. Bloat dimension slightly increased despite splitting functions

After splitting the 112-line `ingest_records` into three functions (~30-50 lines
each), the bloat loss went from 0.176 to 0.202 -- a regression! The geometric
mean of per-function scores changed because I added more functions (each
contributing to the product). The tool penalized me for having more functions
even though each one was shorter.

**Suggestion:** This might be an expected trade-off (bloat vs. code_economy),
but it's surprising for a user who just followed the heatmap's advice to fix
bloat. The heatmap said "ingest_records: 112 lines, bloat" but fixing it made
bloat worse. Consider adding a note about this trade-off dynamic.

---

## Pain Points in the CLI / Output Format

1. **The heatmap bar chart is not very useful.** The `@@........` / `..........`
   visual bars all look the same for small values. At epoch 2, every entry shows
   `..........` (all dots) because the individual responsibilities are so small.
   The bar chart only has visual contrast for the very worst offenders.

2. **No `--format` flag on heatmap.** The heatmap outputs a human-readable
   table, but there's no JSON option for programmatic consumption. You have to
   use `--agent` on analyze to get heatmap data as JSON.

3. **`diff --compliance` is misleading naming.** The `--compliance` flag on
   `diff` doesn't compare against a compliance policy -- it just produces a
   structured assessment. The word "compliance" implies policy enforcement, but
   what the flag actually does is classify deltas as improved/regressed/unchanged.
   Maybe `--assess` or `--summary` would be clearer.

4. **No way to diff more than two epochs.** For a multi-epoch run, you want to
   see the trajectory. Having to run N-1 diffs manually is tedious. A
   `descendit trend epoch0.json epoch1.json epoch2.json epoch3.json` would be
   nice.

5. **Snapshot files are large.** The raw JSON snapshot for a ~500 line crate is
   22KB. For larger codebases, this could become unwieldy. There is no compact
   snapshot format.

---

## What Information Was Missing from the README

1. **How to avoid Cargo build output polluting JSON snapshots.** The README shows
   `descendit analyze src/ > epoch0.json` assuming `descendit` is on PATH. When
   using `cargo run --`, you need `cargo run -q` or the snapshot is corrupted.

2. **What `responsibility` means in heatmap output.** The README never defines
   this term.

3. **Trade-off dynamics between dimensions.** Splitting a bloated function
   improves bloat but can worsen code_economy (more private helpers) and even
   worsen bloat (geometric mean with more terms). The README should acknowledge
   these trade-offs.

4. **What `coupling_density: 0.5` means for the sample analysis.** The semantic
   section showed `coupling_edge_count: 1` and `coupling_module_count: 2` but
   didn't explain how that becomes a density of 0.5.

5. **Whether `Default` trait impls count as public or private.** They are in
   `impl Default for X` blocks and descendit treats them as private (non-pub
   `fn default()`), which affects code_economy. This is surprising since Default
   impls are part of the public API.

6. **The `--top N` flag only works with `--agent`.** If you try
   `descendit heatmap ... --top 10`, it works. But `descendit analyze ... --top 5`
   without `--agent` silently ignores `--top`. The README doesn't clarify this.

---

## Suggestions for Simplification

1. **Add a `descendit fix` or `descendit suggest` command** that outputs
   concrete refactoring suggestions based on the heatmap. E.g.:
   - "config.rs:4 PipelineConfig: 15 boolean fields. Consider grouping related
     flags into sub-structs or using an enum."
   - "ingest.rs:22 ingest_records: 112 lines. Consider extracting phases into
     separate functions."

2. **Add a `descendit loop` command** that runs analyze, saves snapshot, and
   diffs against previous in one step. The manual
   `analyze > epoch0.json` + `analyze > epoch1.json` + `diff epoch0 epoch1`
   dance is the most common workflow and should be a single command.

3. **Default to `--agent` when stdout is a TTY** and the user is clearly in an
   interactive session. The raw JSON dump is never what an interactive user wants.
   Or provide a human-readable summary mode by default.

4. **Consider a `--dimension` filter for heatmap.** When I know I want to fix
   state_cardinality, I don't need to see bloat entries. E.g.:
   `descendit heatmap src/ --dimension state_cardinality --top 10`

5. **Add epoch tracking built-in.** Instead of manually managing `epoch0.json`,
   `epoch1.json`, etc., descendit could maintain a `.descendit/` directory with
   timestamped snapshots and a `descendit history` command to show the
   trajectory.

---

## Errors and Unexpected Behavior

1. **Corrupted snapshot from `cargo run` stdout pollution** (described above).
   The `diff` error message `expected value at line 1 column 5` doesn't hint at
   the actual problem (non-JSON prefix in the file).

2. **Semantic analysis succeeded unexpectedly.** Running
   `descendit analyze <path>` without `--semantic off` on a crate without a
   proper workspace produced semantic results (coupling_density: 0.5) instead of
   failing or warning. This made me unsure whether I even needed `--semantic off`.

3. **No warning when all hotspot responsibilities are tiny.** At epoch 2-3, the
   heatmap shows entries with responsibility 0.005-0.015, and the bar chart is
   all dots. There's no indication that "you've reached diminishing returns, the
   remaining items are negligible." The convergence criterion is only documented
   in the README, not enforced or signaled by the tool.

4. **`code_economy` treats `Default::default()` impls as private functions.**
   These are trait implementations that are effectively public API (they're
   callable by anyone with access to the type), but descendit counts them as
   private because `fn default()` lacks the `pub` keyword. This inflates the
   overhead ratio.

---

## Verdict

descendit is genuinely useful for iterative code quality improvement. The
analyze -> heatmap -> fix -> diff loop works, and the `--agent` mode is excellent
for LLM-driven workflows. The main friction points are in the snapshot management
workflow (pollution, manual epoch tracking) and the lack of qualitative guidance
(what scores are "good enough", when to stop, trade-off awareness).

The tool is approximately 80% of the way to a great first-time experience. The
remaining 20% is documentation clarity, a few CLI ergonomics fixes, and built-in
convergence detection.
