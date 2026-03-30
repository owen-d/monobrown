# descendit guide

Deterministic structural metrics and loss scoring for Rust code, inspired by
ML loss functions. descendit treats code quality dimensions as continuous
signals and composes them into a single scalar loss that monotonically
decreases as code improves.

## Installation

```
cargo install descendit
```

The `semantic` feature (enabled by default) includes the `descendit-ra` crate
for rust-analyzer-powered coupling analysis. To build without it:

```
cargo install descendit --no-default-features
```

## Core workflow

```
analyze  -->  diff  -->  comply
   |                       |
   +------> heatmap <------+
```

1. **Analyze** -- capture a metrics snapshot of your crate or directory.
2. **Diff** -- compare two snapshots to see what improved or regressed.
3. **Comply** -- score a snapshot against a compliance policy.
4. **Heatmap** -- drill into per-function loss attribution to find hotspots.

## Subcommands

### analyze

Scan source code and produce a raw metrics snapshot.

```
descendit analyze <paths...> [options]
```

| Flag | Effect |
|------|--------|
| `--compliance` | Output compliance report instead of raw metrics |
| `--agent` | Agent-friendly compact JSON: composite loss, per-dimension losses, top heatmap items |
| `--top N` | Number of top heatmap items with `--agent` (default 10) |
| `--loss-vector` | Output as structured loss vector |
| `--summary-only` | Only print the summary section |
| `--policy <file>` | Custom compliance policy JSON |
| `--semantic require\|auto\|off` | Semantic enrichment mode (default: require) |
| `--semantic-path <file>` | Provide pre-generated semantic data JSON |

Examples:

```
descendit analyze src/
descendit analyze src/ --compliance
descendit analyze src/ --agent --top 5
descendit analyze src/ --semantic off --loss-vector
```

### diff

Compare two analysis snapshots and show what changed.

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

### comply

Score a saved analysis snapshot against a compliance policy.

```
descendit comply <analysis.json> [options]
```

| Flag | Effect |
|------|--------|
| `--policy <file>` | Custom compliance policy JSON |
| `--semantic require\|auto\|off` | Semantic data mode (default: require) |
| `--semantic-path <file>` | Path to semantic data JSON |

Examples:

```
descendit comply analysis.json
descendit comply analysis.json --policy strict.json
descendit comply analysis.json --semantic off
```

### heatmap

Drill down into which functions and types contribute most to loss.

```
descendit heatmap <paths...> [options]
```

| Flag | Effect |
|------|--------|
| `--tree` | Render as hierarchical rollup tree instead of flat list |
| `--json` | Output as JSON |
| `--policy <file>` | Custom compliance policy JSON |
| `--semantic require\|auto\|off` | Semantic enrichment mode (default: require) |
| `--semantic-path <file>` | Path to semantic data JSON |

Examples:

```
descendit heatmap src/
descendit heatmap src/ --tree
descendit heatmap src/ --json
```

### list

List all available loss dimensions and their descriptions.

```
descendit list [--json]
```

### watch

Watch paths for changes and serve analysis over a Unix socket (Unix only).

```
descendit watch --sock /tmp/descendit.sock <paths...> [--background]
```

The server re-analyzes automatically when source files change. Other commands
can connect to the server via `--sock` for faster repeated queries.

### reap

Shut down a running watch server.

```
descendit reap --sock /tmp/descendit.sock
```

### guide

Print this document.

```
descendit guide
```

### policy

Dump the default compliance policy as JSON.

```
descendit policy --dump-default
```

Use the output as a starting point for custom policy files.

## Loss dimensions

Each dimension produces a score in [0, 1] where 1.0 = perfect compliance.

| Dimension | What it measures |
|-----------|-----------------|
| `duplication` | Ratio of duplicated token sequences across the codebase |
| `state_cardinality` | State-space size of types (enum variants, struct fields, booleans) |
| `bloat` | Function length and complexity beyond thresholds |
| `code_economy` | Overhead ratio: non-test functions / public functions |
| `coupling_density` | Outgoing cross-module edges per module (requires semantic enrichment via descendit-ra) |

## Composite loss

```
composite_loss = 1 - geometric_mean(dimension_scores)
```

- Range: [0, 1]
- 0.0 = perfect (all dimensions score 1.0)
- 1.0 = worst possible

The geometric mean penalizes outliers: one badly-scoring dimension drags the
composite down more than an arithmetic mean would. This incentivizes balanced
improvement across all dimensions.

The default scalarization is geometric mean. Custom policies can switch to
arithmetic mean via the `objective_scalarization` field.

## Semantic enrichment

The `coupling_density` dimension requires cross-module dependency data that
cannot be extracted from syntax alone. descendit delegates this to
`descendit-ra`, which uses rust-analyzer internals.

Control semantic enrichment with the `--semantic` flag:

| Mode | Behavior |
|------|----------|
| `require` (default) | Run descendit-ra; fail if unavailable |
| `auto` | Try descendit-ra; fall back to syntax-only if it fails |
| `off` | Skip semantic analysis entirely |

You can also provide pre-generated semantic data with `--semantic-path <file>`
to avoid re-running the backend.

## Gradient-descent refactoring workflow

descendit is designed for iterative, measurable refactoring modeled after
gradient descent in ML:

1. **Baseline** -- capture the starting state:
   ```
   descendit analyze src/ --compliance > baseline.json
   ```

2. **Identify hotspots** -- find the highest-loss items:
   ```
   descendit heatmap src/ --tree
   ```

3. **Refactor** -- fix the top contributor(s).

4. **Re-score** -- measure the delta:
   ```
   descendit analyze src/ --compliance > current.json
   descendit diff baseline.json current.json --compliance
   ```

5. **Converge** -- repeat steps 2-4. Stop when the composite loss delta
   between iterations is less than 0.005, indicating diminishing returns.

For agent-driven workflows, the `--agent` flag on `analyze` produces compact
JSON suitable for programmatic consumption:

```
descendit analyze src/ --agent
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
descendit comply analysis.json --policy policy.json
```
