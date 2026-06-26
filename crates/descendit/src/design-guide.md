# descendit-design agent guide

`descendit-design` is for speculative Rust design analysis. It consumes the
same semantic facts as `descendit`, but it does not score the loss-gradient
loop and it should not be treated as an automatic refactoring instruction.

## Shape

```
Rust crate
  -> semantic facts
  -> design model
  -> candidate rewrites
  -> human review
```

Use `descendit` when the question is "did the code get measurably better?"
Use `descendit-design` when the question is "does this code have a simpler
shape worth considering?"

## Type/trait discovery

```
descendit-design type-trait discover --query q.json
```

The command builds a scoped type/trait graph, hides standard traits by default,
and reports candidate rewrites with before/after diagrams, source refs, and
rewrite notes.

Pipe the query through stdin when an agent is synthesizing it:

```
printf '%s\n' '{"path":"crates/my-crate","emit":"text"}' \
  | descendit-design type-trait discover --query -
```

## Query fields

```json
{
  "path": "crates/my-crate",
  "semantic_path": "target/descendit/semantic.json",
  "scope": { "name": "Repository|Service" },
  "include_one_hop": true,
  "ignored_traits": ["Debug", "Clone"],
  "emit": "text"
}
```

- `path`: crate, workspace, directory, or file used to find the nearest
  `Cargo.toml` when semantic facts must be generated.
- `semantic_path`: optional raw semantic JSON. When present, the tool loads
  this file and skips rust-analyzer.
- `scope`: optional selector for the graph slice. It can match crate names,
  path prefixes, module prefixes, node-name regexes, or compose those with
  `any`, `all`, and `not`.
- `include_one_hop`: keeps direct neighbors around scoped local nodes. This is
  usually what makes the diagram readable without losing context.
- `ignored_traits`: override the default hidden-trait set. Put every trait you
  want hidden in this list.
- `emit`: `text` for agent-readable reports or `json` for downstream tooling.

## Reusing expensive RA work

There are two reuse paths:

1. Saved semantic facts: set `semantic_path` in the query. This is the strongest
   reuse mode because the design command does not invoke rust-analyzer at all.
   It expects raw descendit semantic JSON, not an `analyze` loss snapshot.
2. Warm server: run `descendit watch` and pass `--sock` to `descendit-design`.
   This reuses the watch server's warm rust-analyzer session and asks only for
   the type/trait analysis domain.

```
descendit watch --sock /tmp/descendit.sock crates/my-crate
descendit-design --sock /tmp/descendit.sock type-trait discover --query q.json
```

The warm-server path avoids repeated cold starts, but it is not a durable
on-disk cache. A dedicated semantic dump/cache command would be a separate
addition.

## Output

Text output is optimized for review:

- initial graph: the scoped type/trait graph used for discovery
- candidates: ranked rewrite opportunities
- before/after diagrams: what the graph would look like after the rewrite
- plan: concrete operations the rewrite suggests
- sources: source locations backing each candidate

JSON output preserves the same report shape for tools that want to render,
filter, or compare candidates themselves.
