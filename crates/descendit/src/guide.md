# descendit agent guide

Deterministic structural metrics and loss scoring for Rust code, inspired by
ML loss functions. descendit treats code quality dimensions as continuous
signals and composes them into a single scalar loss that monotonically
decreases as code improves.

## Quick start

```
descendit analyze src/ --agent --top 5
```

This prints composite loss, per-dimension scores, and the top 5 hotspots.
Semantic analysis via rust-analyzer runs automatically.

## Installation

```
cargo install descendit
```

Requires Rust 1.85+. Includes rust-analyzer integration for cross-module
coupling analysis.

## Core workflow

```
analyze  -->  diff
   |              |
   +-> heatmap <--+
```

1. **Analyze** -- capture a metrics snapshot of your crate or directory.
2. **Diff** -- compare two snapshots to see what improved or regressed.
3. **Comply** -- score a snapshot against a compliance policy.
4. **Heatmap** -- drill into per-function loss attribution to find hotspots.

All commands accept `--sock` to connect to a running `watch` server. Use
`watch` for iterative work — it keeps a persistent rust-analyzer session so
repeated analysis avoids cold starts.

## Subcommands

### analyze

Scan source code and produce a raw metrics snapshot.

```
descendit analyze <paths...> [options]
```

| Flag | Effect |
|------|--------|
| `--agent` | Compact JSON: composite loss, per-dimension losses, top heatmap items |
| `--top N` | Number of top heatmap items with `--agent` (default 10) |
| `--semantic-path <file>` | Pre-generated semantic data JSON (skips RA) |

Without `--agent`, outputs the full raw metrics snapshot (used as input
for `diff`).

Examples:

```
descendit analyze src/                      # raw snapshot (pipe to file for diff)
descendit analyze src/ --agent --top 5      # compact summary
```

### diff

Compare two analysis snapshots and show what changed. The input files are
the raw JSON output of `analyze` (without `--agent`, `--compliance`, or
`--loss-vector`). Semantic data is already baked into the snapshots.

```
descendit diff <baseline.json> <current.json> [options]
```

| Flag | Effect |
|------|--------|
| `--compliance` | Compare at the compliance/loss level (composite + per-dimension deltas) |
| `--loss-vector` | Output as structured loss vector |
| `--heatmap` | Show heatmap item changes between snapshots |
| `--json` | Output heatmap diff as JSON |
| `--policy <file>` | Custom compliance policy JSON |
| `--semantic-path <file>` | Path to semantic data JSON |

Examples:

```
descendit diff baseline.json current.json --compliance
descendit diff baseline.json current.json --heatmap --json
```

### heatmap

Drill down into which functions and types contribute most to loss.

```
descendit heatmap <paths...> [options]
```

| Flag | Effect |
|------|--------|
| `--tree` | Render as hierarchical rollup tree instead of flat list |
| `--top N` | Limit output to top N entries by responsibility |
| `--json` | Output as JSON |
| `--policy <file>` | Custom compliance policy JSON |
| `--semantic-path <file>` | Path to semantic data JSON (skips RA) |

Examples:

```
descendit heatmap src/
descendit heatmap src/ --tree
descendit heatmap src/ --top 20
descendit heatmap src/ --json
```

### list

List all available loss dimensions and their descriptions.

```
descendit list [--json]
```

### watch

Start a persistent analysis server over a Unix socket (Unix only). This is
the preferred mode for iterative refactoring — rust-analyzer stays warm and
re-scores are near-instant.

```
descendit watch --sock /tmp/descendit.sock <paths...>
```

The server re-analyzes automatically when source files change. Other commands
connect via `--sock` for fast repeated queries:

```
descendit analyze src/ --sock /tmp/descendit.sock --agent --top 5
descendit heatmap src/ --sock /tmp/descendit.sock --top 10
```

### reap

Shut down a running watch server.

```
descendit reap --sock /tmp/descendit.sock
```

### guide

Print this document.

```
descendit agent guide
```

### policy

Dump the default compliance policy as JSON.

```
descendit policy --dump-default
```

Use the output as a starting point for custom policy files.

## Loss dimensions

Each dimension produces a score in [0, 1] where 1.0 = perfect compliance.

| Dimension | What it measures | How to reduce |
|-----------|-----------------|---------------|
| `duplication` | Fraction of functions in structural duplicate groups | Extract shared logic into a common function |
| `state_cardinality` | 2^(bool_fields + option_fields) per type; geometric mean of log2 values | Split bool-heavy structs into focused sub-structs |
| `bloat` | Function line count beyond threshold (default 35 lines) | Extract phases into smaller functions |
| `code_economy` | Overhead ratio: non-test, non-pub functions / pub functions | Make meaningful helpers public, or reduce unnecessary private wrappers |
| `coupling_density` | Outgoing cross-module call edges per module (via rust-analyzer) | Reduce cross-module dependencies; colocate tightly coupled code |

## Composite loss

```
composite_loss = 1 - geometric_mean(dimension_scores)
```

- Range: [0, 1]
- 0.0 = perfect (all dimensions score 1.0)
- 1.0 = worst possible

Rough interpretation:

| Composite loss | Quality |
|---------------|---------|
| 0.00 - 0.05 | Excellent — diminishing returns on further improvement |
| 0.05 - 0.15 | Good — typical well-maintained code |
| 0.15 - 0.30 | Room for improvement — targeted refactoring will help |
| 0.30+ | Significant structural issues — start with the heatmap |

The geometric mean penalizes outliers: one badly-scoring dimension drags the
composite down more than an arithmetic mean would. This incentivizes balanced
improvement across all dimensions.

Dimensions marked `not_measured` are excluded from the geometric mean — they
do not inflate or deflate the score.

The default scalarization is geometric mean. Custom policies can switch to
arithmetic mean via the `objective_scalarization` field.

## Semantic enrichment

The `coupling_density` dimension requires cross-module dependency data that
cannot be extracted from syntax alone. descendit delegates this to
`descendit-ra`, which uses rust-analyzer internals. Semantic analysis runs
automatically on every invocation.

To avoid re-running the backend, provide pre-generated semantic data with
`--semantic-path <file>`. Use `watch` mode for fast repeated analysis.

## Gradient-descent refactoring workflow

descendit is designed for iterative, measurable refactoring modeled after
gradient descent in ML. Use `watch` mode so rust-analyzer stays warm and
re-scores are near-instant:

```
# Start the analysis server
descendit watch --sock /tmp/descendit.sock src/

# Epoch 0: baseline
descendit analyze src/ --sock /tmp/descendit.sock > epoch0.json
descendit heatmap src/ --sock /tmp/descendit.sock --top 10

# ... refactor the top hotspot ...

# Epoch 1: measure improvement
descendit analyze src/ --sock /tmp/descendit.sock > epoch1.json
descendit diff epoch0.json epoch1.json --compliance

# Repeat until convergence (|delta| < 0.005 between epochs)

# Shut down the server when done
descendit reap --sock /tmp/descendit.sock
```

A typical convergence run:

| Epoch | Composite loss | Delta | Action |
|-------|---------------|-------|--------|
| 0 | 0.142 | — | baseline |
| 1 | 0.098 | -0.044 | split bloated function |
| 2 | 0.071 | -0.027 | extract duplicated logic |
| 3 | 0.065 | -0.006 | reduce state cardinality |
| 4 | 0.063 | -0.002 | diminishing returns, stop |

Most improvement lands in epochs 1-2. By epoch 4+, diminishing returns.

The raw JSON output of `analyze` (no `--agent` or `--compliance`) is the
snapshot format expected by `diff`.

Common fixes by dimension:
- **state_cardinality**: split bool-heavy structs into focused sub-structs
- **bloat**: extract function phases into smaller functions
- **duplication**: extract shared logic into a common function
- **code_economy**: make meaningful helpers public; remove dead private code

**Trade-off awareness:** Splitting a bloated function into helpers can worsen
code_economy (more private functions). Making those helpers `pub` fixes
code_economy but changes the API surface. These trade-offs are inherent to
multi-dimensional scoring — the composite loss reflects the net effect.

For quick one-shot analysis without a watch server:

```
descendit analyze src/ --agent --top 10
```

## Policy customization

Dump the default policy:

```
descendit policy --dump-default > policy.json
```

The policy JSON controls:

- **Directional scales** -- how many units of raw metric change map to one
  normalized unit (e.g., `code_economy_log2_overhead`, `bloat_log2_lines`).
- **Normalization** -- cohort-aware mean/stddev or quartile/IQR normalizers.
- **Calibration** -- per-dimension calibration adjustments.
- **Aggregation** -- how per-item scores roll up into dimension scores
  (geometric mean, arithmetic mean, power mean) and how dimension scores
  scalarize into the composite (geometric or arithmetic mean).

Apply a custom policy:

```
descendit analyze src/ --compliance --policy policy.json
```
