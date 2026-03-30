# descendit

Deterministic structural metrics and loss scoring for Rust code.

`descendit` parses Rust source with `syn`, extracts quantifiable structural metrics, and scores them through a pipeline inspired by ML loss functions. The same source always produces the same scores — deterministic by design.

Built for agent-driven refactoring loops: score, identify hotspots, refactor, re-score, converge.

## Install

```bash
cargo install descendit
```

Requires Rust 1.85+ (edition 2024).

## Quick Start

```bash
# Score a crate
descendit analyze src/ --compliance

# Agent-friendly compact output
descendit analyze src/ --agent

# Find what to fix first
descendit heatmap src/

# Baseline → refactor → compare
descendit analyze src/ > before.json
# ... make changes ...
descendit analyze src/ > after.json
descendit diff before.json after.json --compliance
```

## Loss Dimensions

| Dimension | Scope | What it measures |
|-----------|-------|-----------------|
| `duplication` | global | Fraction of functions in structural duplicate groups |
| `state_cardinality` | per-type, per-function | Geometric mean of log2 field cardinality |
| `bloat` | per-function | Geometric mean of line counts (threshold: 35 lines) |
| `code_economy` | global | Overhead ratio (non-test functions / public functions) |
| `coupling_density` | per-module | Cross-module call edges (requires semantic enrichment) |

**Composite loss** = `1 - geometric_mean(dimension_scores)`. Range [0, 1], where 0.0 is perfect.

## CLI Reference

### Core commands

| Command | Purpose |
|---------|---------|
| `analyze <paths...>` | Extract metrics, optionally score with `--compliance`, `--agent`, `--loss-vector` |
| `diff <base.json> <cur.json>` | Compare snapshots with `--compliance`, `--heatmap` |
| `comply <analysis.json>` | Re-score a saved snapshot against a different `--policy` |
| `heatmap <paths...>` | Rank functions/types by loss responsibility (`--tree` for hierarchy) |
| `list` | Print all loss dimensions and their formulas |

### Semantic analysis

| Command | Purpose |
|---------|---------|
| `watch --sock <path> <paths...>` | Persistent rust-analyzer server for fast incremental analysis |
| `reap --sock <path>` | Shut down a running watch server |

Semantic enrichment mode: `--semantic require` (default), `auto`, or `off`.

### Utilities

| Command | Purpose |
|---------|---------|
| `guide` | Emit a self-contained markdown guide for LLM consumption |
| `policy --dump-default` | Dump the default compliance policy as JSON |

## Semantic Enrichment

By default, `descendit` runs rust-analyzer to extract cross-module call edges and refined type cardinalities. This enables the `coupling_density` dimension.

The `--semantic` flag controls this:
- `require` (default) — run RA; fail if it can't
- `auto` — try RA; fall back to syntactic-only on failure
- `off` — skip RA entirely

For repeated runs, use `watch` to keep a persistent RA session:

```bash
descendit watch --sock /tmp/descendit.sock src/ &
descendit analyze src/ --sock /tmp/descendit.sock --compliance
descendit reap --sock /tmp/descendit.sock
```

## Policy Customization

Scoring thresholds and aggregation strategies are configurable via JSON policy files:

```bash
# See the defaults
descendit policy --dump-default > my-policy.json

# Edit thresholds, then use it
descendit analyze src/ --compliance --policy my-policy.json
```

## Gradient-Descent Refactoring

The intended workflow:

1. **Baseline**: `descendit analyze src/ --agent` — get composite loss + top hotspots
2. **Identify**: `descendit heatmap src/ --tree` — find highest-responsibility items
3. **Refactor**: Fix the top hotspot (duplication is usually highest-ROI)
4. **Re-score**: `descendit analyze src/ --agent` — check delta
5. **Converge**: Repeat until `|delta| < 0.005` between epochs

Most improvement lands in epochs 1-2. By epoch 4+, you're in diminishing returns.

## License

Apache 2.0 — see [LICENSE](../LICENSE).
