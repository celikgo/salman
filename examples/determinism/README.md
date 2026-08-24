# The determinism reference

`hazards.st` is the program `.github/workflows/determinism.yml` runs on Linux, macOS and
Windows to check rule 6: *same project, same inputs, same seed, identical trace, bit for bit.*

It is a **CI fixture, not a tutorial.** If you have never written Structured Text, read
[`../conveyor/`](../conveyor/) instead — that is a machine, with a start-stop station, a part
counter and a jam timer. This one is a list of hazards that drives itself.

```bash
salman run examples/determinism/hazards.st --scans 1000 \
    --record Scan,Wrapping,Drift,Third,NotANumber,Infinite,NegZero,Elapsed,Phase,Alternating,Second
```

## What each column is for

Every variable earns its place by being a hazard
[`ADR-0005`](../../docs/adr/ADR-0005-determinism.md) names. One that stops being a hazard
should be deleted rather than kept to make the trace longer.

| Column | The hazard |
|---|---|
| `Scan` | A counter, so no column is constant by accident and a truncated trace is obvious |
| `Wrapping` | `SINT` overflow wraps — a *salman policy*, not a standard requirement (`docs/CONFORMANCE.md` policy 13) |
| `Drift` | A `REAL` accumulating `0.1`, which has no exact binary representation. By scan 1000 it reads `99.9990463256836` rather than `100.0`, and that number is the fingerprint of 32-bit float arithmetic plus shortest-round-trip formatting |
| `Third` | `LREAL` division, which IEC 61131-3:2013 references IEEE 754 for and Rust specifies exactly |
| `NotANumber` | `0.0 / 0.0`. On aarch64 that yields `0x7ff8…`; on x86-64 the default quiet NaN is the negative-signed indefinite, `0xfff8…`. salman canonicalises on entry to a `Value` |
| `Infinite` | Overflow to `inf` |
| `NegZero` | `-0.0`, which salman deliberately **preserves** — unlike NaN it is portable, and `1.0 / -0.0` is `-inf` |
| `Elapsed` | `TIME` arithmetic, rendered back as an IEC duration literal |
| `Phase`, `Alternating` | A little control flow, so the trace is not one straight line of assignments |
| `Second` | A second `PROGRAM`, so the trace has two tasks in it. With no `CONFIGURATION` each gets a freewheeling task in declaration order (policy 18), so both are released at the same instant and the `task` column alternates. Which row comes first is broken by `(next_release_ns, priority, index)` in `task.rs` — if that order ever came from iterating a collection, the rows would swap and the fingerprint would move |

## Why the gate compares fingerprints and not this text

The three float columns all **render identically on every architecture**. `NaN` is `NaN`
whatever bits are underneath it. So a rendered trace cannot tell you whether canonicalisation
is still working.

The fingerprint can. It is SHA-256 over `Value::write_canonical_bytes` — a tagged binary
encoding — rather than over the rendered text, so a NaN that stopped being canonicalised
changes the fingerprint while the column still reads `NaN`.

That is why `ADR-0005` rejected comparing traces as rendered text and put it in one sentence:
**salman renders text for humans and hashes bytes for the gate.** The workflow asserts on the
fingerprint and shows a text diff only when it has already decided something is wrong, so that
a human has somewhere to start.

## What this does not cover

- **Transcendentals.** `sin`, `cos`, `exp`, `ln` and the rest are banned by `clippy.toml`
  because `std` delegates them to the platform libm. salman implements no standard functions
  at all yet, so there is nothing to exercise and nothing to compare.
- **`**` exponentiation**, which the compiler refuses by name under `U0501`.
- **Anything ordered by a hash map**, directly. There are no `HashMap`s in the workspace to
  exercise; `clippy.toml` denies the type outright. The `Second` column covers the closest
  observable consequence — two tasks released at the same instant, whose row order would swap
  if the tie-break ever came from iteration order rather than from declaration order.
- **Threads.** The runtime is single-threaded by design, so there is no scheduling to vary.

Each of those is absent from this file because it is absent from salman, not because it was
judged unimportant. When one arrives, it gets a column here in the same commit.

## Not a safety function

Nothing in `hazards.st` controls anything, and salman is not certified under any functional
safety standard. See [`../../LEGAL.md`](../../LEGAL.md).
