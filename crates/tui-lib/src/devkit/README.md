# TUI DevKit Principles

A data-driven development kit for building TUI applications through an agent-human
feedback loop. Lives in `mb-tui` as the `devkit` module.

## Core Idea

Everything is a **scenario** — a value that describes a state to render. Scenarios are
the single source of truth. Tests render them headlessly. The playground renders them
visually. Captured interactions become new scenarios. One data model, three modes.

## The Three Modes

### Test Mode (agent-facing)

Render scenarios via `ratatui::TestBackend` to produce monochromatic text grids.
Snapshot with `insta`. The agent iterates here autonomously: change code → run tests →
read snapshot diffs → repeat. No human in the loop.

**What it catches:** Layout, structure, spatial regressions, text content, proportions.

**What it doesn't catch:** Color, visual feel, aesthetic quality. That's fine — color
logic should be isolated to style functions testable with simple unit assertions.

### Playground Mode (human-facing)

Render the same scenario catalog with a real terminal backend. The human sees color,
proportion, gestalt — things an agent can't judge from text. Starts as a non-interactive
gallery (cycle through scenarios with arrow keys). Grows into interactive exploration
over time.

**The human's job here is verification, not construction.** The agent builds; the human
scans and gives targeted feedback.

### Capture Mode (bridge)

In playground, each state transition (keystroke, resize, mutation) produces a derived
scenario. These transitions can be captured as a stream and fed back into the catalog —
turning interactive exploration into regression test data.

The loop is self-enriching: exploration → captured scenarios → richer test coverage →
more agent autonomy.

## Design Principles

### 1. Scenarios Are Rust Data

Scenarios use your domain types directly. No serialization format (TOML, JSON, YAML).
This gives you type safety, IDE support, and zero translation cost.

```rust
use mb_tui::devkit::{Scenario, ScenarioCatalog};

// Scenarios are declarative data construction — no assertions, no rendering, no side effects.
let scenario = Scenario {
    name: "loading",
    description: "Standard loading state",
    state: MyAppState { loading: true, items: vec![] },
};

// The catalog pairs scenarios with a render function.
let mut catalog = ScenarioCatalog::new(|state: &MyAppState, area, buf| {
    // render state into buf
});
catalog.add(scenario);

// Agent feedback loop: render all scenarios to snapshots.
catalog.assert_all_snapshots(80, 24);
```

### 2. The Kit Is Domain-Agnostic

The devkit does not know about flamegraphs, compliance reports, or any specific domain.
It provides:

- `Scenario<S>` struct for naming and describing state
- `ScenarioCatalog<S>` for collecting scenarios and pairing them with a render function
- `Surface` for off-screen rendering (TestBackend wrapper)
- Text extraction (`buffer_to_text`, `buffer_to_styled_text`, `buffer_to_ansi`)
- Snapshot integration via insta

Consumers bring their own types, their own rendering function, their own scenarios.

### 3. Separation of Concerns

| Concern | Owns it | Does NOT own it |
|---------|---------|-----------------|
| Scenario data | Consumer crate | DevKit |
| Render function | Consumer crate | DevKit |
| TestBackend plumbing | DevKit | Consumer crate |
| Snapshot management | DevKit | Consumer crate |
| Playground shell | DevKit | Consumer crate |
| Domain types | Consumer crate | DevKit |

### 4. Agent Autonomy Is the Primary Goal

The devkit exists so an agent can iterate on TUI rendering without human involvement.
Design decisions should be evaluated against: "Does this let the agent do more without
asking the human?"

- Text snapshots are readable by agents → agent can assess layout changes
- Scenario catalog is enumerable → agent can run all cases
- Snapshot diffs are precise → agent can see exactly what changed
- New scenarios are just Rust data → agent can create them

### 5. Human Involvement Is Targeted

The human participates at two points:

1. **Visual verification** — scan the playground, confirm things look right
2. **Feedback** — "the colors are wrong on scenario 3", "this feels cramped"

The human should never need to: run the full app, resize terminals manually, construct
test data, or interpret ANSI escape codes.

### 6. Composition Over Configuration

Scenarios compose from building blocks. Edge cases are built by combining simple
scenarios. The data model should make this natural — not through a configuration DSL,
but through Rust's normal composition tools (functions, builders, combinators).

### 7. Start Simple, Accrete

Do not build later stages until earlier ones are proven.

## Current Architecture

```
mb-tui/src/devkit/
├── mod.rs          // module root, re-exports
├── scenario.rs     // Scenario<S>, ScenarioCatalog<S>
├── surface.rs      // Surface (off-screen render target)
└── text.rs         // buffer → text/styled_text/ansi extraction

Consumer tests (e.g., mb-tui's own spinner test):
tests/
├── devkit_spinner.rs           // scenario catalog + snapshot assertions
└── devkit_spinner/snapshots/   // insta snapshot files
```

## The Feedback Loop

```
 ┌─────────────────────────────────────────────────────┐
 │                   scenario catalog                   │
 │              (Rust data, checked in git)              │
 └──────┬────────────────────────┬─────────────────────┘
        │                        │
        ▼                        ▼
 ┌──────────────┐        ┌──────────────┐
 │  test mode   │        │  playground  │
 │  (agent)     │        │  (human)     │
 │              │        │              │
 │ TestBackend  │        │ real term    │
 │ + insta      │        │ + color      │
 └──────┬───────┘        └──────┬───────┘
        │                       │
        │ snapshot diffs        │ visual feedback
        │                       │ captured transitions
        ▼                       │
 ┌──────────────┐               │
 │ agent iterates│◄─────────────┘
 │ autonomously  │  new scenarios
 └───────────────┘  flow back in
```

## What Success Looks Like

1. An agent can pick up a TUI rendering task, iterate through multiple rounds of
   changes, and produce correct output — verified by snapshots — without any human
   interaction.

2. A human can run the playground, scan all scenarios in under a minute, and give
   precise feedback ("scenario X, the label is clipped") that the agent can act on.

3. Adding a new scenario is a 5-line Rust function. Adding a new TUI consumer is
   providing a render function and a set of `Scenario<S>` values.

4. The test catalog grows organically as the playground captures real interactions,
   making regressions progressively harder to introduce.
