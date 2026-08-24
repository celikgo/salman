---
name: preserving-determinism
description: What salman's determinism promise actually covers, which half of it CI enforces and which half it does not, the constructs that would break a trace (hash-map iteration order, float formatting, wall-clock reads, thread scheduling, transcendental functions, NaN bit patterns), and how to run the byte-for-byte trace comparison locally. Use when touching salman-vm, the trace format, the virtual clock, anything ordered or hashed, anything that formats a float, or when a golden-trace test fails, when the determinism workflow goes red, or when writing a claim about reproducibility.
---

# Determinism

Rule 6 in `README.md`: *same project, same inputs, same seed, identical trace, bit for bit.*

That promise is what makes a golden-trace test worth committing. If a simulation produces a
slightly different trace on a reviewer's laptop than on the build agent, the test tells you
nothing and everyone learns to ignore it.

`docs/adr/ADR-0005-determinism.md` is the decision record. Read it before arguing with
anything here — it lists the seven mechanisms and, more usefully, the failure modes that
motivated each. This skill is the operating manual.

## What the gate actually is

`.github/workflows/determinism.yml` checks rule 6 in two halves, and they answer different
questions. Both are real; neither is the whole claim.

**Half one: each platform against a committed file.** The `cross-platform-tests` job runs
`cargo test --workspace --all-features` on `ubuntu-latest`, `macos-latest` and
`windows-latest` with `fail-fast: false`. That suite contains
`the_recorded_trace_matches_the_committed_golden_file`
(`crates/salman-cli/tests/conveyor_example.rs`), which re-renders the conveyor's trace and
asserts it equals the committed `examples/conveyor/conveyor.trace`, string for string.
`.gitattributes` marks `*.trace` as `-text` so git never rewrites those bytes,
`a_golden_trace_file_contains_no_carriage_returns` asserts that git has not, and Rust's
`read_to_string` translates no line endings. A platform whose runtime disagreed with that file
fails there.

**Half two: the three platforms against each other.** The same job then records
`examples/determinism/hazards.st` for 1000 scans and uploads the trace as a per-OS artefact.
The `compare` job downloads all three and asserts their **fingerprints** are equal. That is
the half that answers *which* platform disagreed, which comparing each one against a file
cannot.

### Why the assertion is on the fingerprint

`ADR-0005` rejected comparing traces as rendered text, and put the reason in one sentence:
**salman renders text for humans and hashes bytes for the gate.** Text puts Rust's float
formatting inside the determinism promise — platform-identical today, no cross-version
guarantee, and it has changed before, so a toolchain bump could break the gate for a reason
unrelated to the simulation.

It is also weaker exactly where it matters. `examples/determinism/hazards.st` puts a NaN, an
infinity and a negative zero in the trace deliberately, and **all three render identically on
every architecture** — a NaN that stopped being canonicalised would still print `NaN`. The
fingerprint is SHA-256 over `Value::write_canonical_bytes`, so it would change. The job prints
a text diff only once the fingerprints have already disagreed, which is the one job rendered
text has in a gate: telling a person where to look.

### What the reference program covers, and what it cannot

`Drift` is a `REAL` accumulating `0.1`; by scan 1000 it reads `99.9990463256836` rather than
`100.0`, and that number is a fingerprint of 32-bit float arithmetic. `Third` is `LREAL`
division. `Wrapping` is `SINT` overflow, which is *salman policy* rather than a standard
requirement. `Elapsed` is `TIME` rendered back as an IEC literal. `examples/determinism/README.md`
has the full table.

It cannot cover transcendentals (banned by `clippy.toml`; salman implements no standard
functions), `**` (refused by the compiler as `U0501`), hash-map ordering (there are no
`HashMap`s to exercise) or thread scheduling (the runtime is single-threaded). Each is absent
because it is absent from salman, not because it was judged unimportant.

### What the gate still does not license

**salman does not claim cross-platform determinism**, and this gate existing does not change
that. `ADR-0005` is explicit: the claim waits "until that job has been green on all three
operating systems for a meaningful period", and one green run is evidence, not a period.
`README.md` rule 6 says the same. Write that the gate exists and runs; do not write that the
premise is settled.

Everything below is what holds determinism up in the code itself.

## What actually enforces it

### 1. The clippy bans — the only gate on the source itself

`clippy.toml` is where the real enforcement lives, and `ci.yml` runs clippy with
`-D warnings`, so a violation fails the build.

- **`disallowed-types`**: `std::collections::HashMap` and `std::collections::HashSet`. The
  stated reason: `RandomState` is seeded per thread and then incremented per instance, so
  iteration order differs from run to run *on one machine*.
- **`disallowed-methods`**: every transcendental on both `f32` and `f64` — `sin`, `cos`,
  `tan`, `asin`, `acos`, `atan`, `atan2`, `exp`, `exp2`, `ln`, `log`, `log2`, `log10`,
  `powf`, `powi`, `cbrt`, `hypot`. `std` delegates these to the platform libm, whose
  precision the Rust documentation says "varies by platform, Rust version, and can even
  differ within the same execution from one invocation to the next". `powi` is banned for a
  different reason: it is not one IEEE operation, and the compiler may expand it into a
  different multiplication tree per target.

The substitutes are `BTreeMap` / `BTreeSet`, and — when salman implements the maths
functions — the pinned pure-Rust `libm` crate. Note what `libm` buys and what it does not:
one pinned implementation, so results do not vary with the host C library. `libm` documents
no bit-exactness guarantee, and ADR-0005 says so rather than overclaiming.

**Current state of the tree, which you can check in one command:**
`grep -rn 'HashMap<\|HashSet<' crates/ --include='*.rs' | wc -l` returns **0**, against 23 for
`BTreeMap<` / `BTreeSet<`. The single `HashSet` construction anywhere in the workspace is
`std::collections::HashSet::new()` inside a test in `crates/salman-core/src/ident.rs`, which
exists precisely to exercise a `Hash` implementation.

If you genuinely need a hash map for lookup only, pass an explicit deterministic hasher and
`#[allow]` the lint **at that site**, with a comment saying why the order cannot escape.
An `#[allow]` at module or crate level is the wrong shape: it silences the next one too.

### 2. NaN canonicalisation

`canonical_f32` and `canonical_f64` in `crates/salman-core/src/value.rs` collapse every NaN
to one quiet NaN with a zero payload — `0x7fc0_0000` and `0x7ff8_0000_0000_0000` — and
`Value::real`, `Value::lreal` and `write_canonical_bytes` all apply them.

This is not theoretical. On aarch64 a run-time `0.0 / 0.0` yields
`0x7ff8_0000_0000_0000`; on x86-64 the default quiet NaN is the negative-signed indefinite,
`0xfff8_0000_0000_0000`. One division would break a byte comparison on its own. ADR-0005
notes this was checked by running it, not reasoned about.

Negative zero is deliberately **preserved**: unlike NaN it is portable, and `1.0 / -0.0` is
`-inf`, which is a real answer a program can depend on.

Tests: `nan_is_canonicalised_so_traces_cannot_differ_between_architectures`,
`negative_zero_is_preserved_because_it_is_portable_and_meaningful`,
`canonical_bytes_are_tagged_so_two_types_holding_one_cannot_collide`.

### 3. The fingerprint is over bytes, not over text

The trace header carries a SHA-256 fingerprint, and it is computed over a **canonical binary
encoding**, not over the rendered text. `Value::write_canonical_bytes` writes a type tag and
then little-endian bytes, so `Value::Int(1)` and `Value::Dint(1)` cannot hash the same.

The hash itself is in-crate: `crates/salman-core/src/hash.rs`, with
`the_published_fips_180_4_vectors_hash_to_their_published_digests` as its known-answer test.

### 4. The clock is virtual, and nothing on the evaluation path reads a real one

`crates/salman-vm/src/clock.rs` holds the simulation clock, with a `ClockMode` and an
`is_deterministic()` predicate; `a_new_virtual_clock_starts_at_zero_and_is_deterministic`
asserts the default.

There is no lint forbidding `SystemTime` or `Instant::now` — that is worth knowing, because
it means this one is held up by review and by the crate boundary, not by a tool. The current
state of the tree is clean and easy to re-check: the only `Instant::now` calls in library
code are in `crates/salman-modbus-net/src/client.rs`, computing socket deadlines. That crate
is the one that reaches a network, it is not on the evaluation path, and a socket timeout
cannot reach a trace. Everything else is in tests and examples.

`crates/salman-vm/src/lib.rs` states the position at the top: *"Nothing in the evaluation
path reads a clock, iterates a hash map, or calls a standard library transcendental
function."* Keep that sentence true.

### 5. Single-threaded, by design

Also from `salman-vm`'s module doc: floating-point addition is not associative, so any
parallel reduction over reals reassociates according to thread scheduling and cannot produce
a reproducible answer. There is no thread pool in the runtime and adding one is not a
performance decision, it is a determinism decision, and it needs an ADR.

### 6. Line endings

`.gitattributes` sets `* text=auto eol=lf` and then exempts golden artefacts from
normalisation entirely: `golden/**`, `examples/**/golden/**`, `*.golden`, `*.trace`, `*.pcap`
are all `-text`. Without that, the determinism gate would quietly become a line-ending test
on Windows. `a_rendered_trace_contains_no_carriage_returns_on_any_platform` in
`crates/salman-vm/src/trace.rs` is the paired assertion.

### 7. The toolchain pin

`rust-toolchain.toml` pins `1.94.1`. The comment says why, and it is the same argument: a
byte comparison across three operating systems is only meaningful if every machine runs the
same compiler. Changing it is a reviewed change. It is not checked by a test — it would be
noticed only by the gate failing.

## What a trace actually looks like

```
# salman trace format 1
# salman version: 0.1.0
# seed: 0
# clock: virtual (reproducible)
# fingerprint: 5a8201e171d849e1768ab2d086bfac66222e57880260ee8905eb0b04f1ddb9a6
# samples: 17
scan	time	task	Motor	Jam_Lamp	Batch_Done	Parts.CV	Progress	State
1	T#0s	0	TRUE	FALSE	FALSE	0	0	1
2	T#1ms	0	TRUE	FALSE	FALSE	0	0	1
```

That is `examples/conveyor/conveyor.trace`, unedited. Six comment lines of header, a
tab-separated column header, then one row per sample: scan number, simulation time as an IEC
duration literal, task index, then one column per recorded signal in the order the `record:`
list gave them.

Everything in the header is either fixed, declared, or derived from the run. Nothing is
ambient — no hostname, no wall-clock timestamp, no path, no user. `a_trace_contains_nothing_ambient`
in `crates/salman-vm/src/trace.rs` is the test that keeps it that way, and it is the test to
extend if you add a header field. **A header field that varies by machine breaks every
golden trace in the repository at once.**

## Reproducing byte-for-byte locally

The workflow does not do this, so you have to. It takes seconds.

```bash
cargo build --release

./target/release/salman run examples/conveyor/conveyor.st \
    --until T#5s --record Motor,State --trace /tmp/a.trace
./target/release/salman run examples/conveyor/conveyor.st \
    --until T#5s --record Motor,State --trace /tmp/b.trace

cmp /tmp/a.trace /tmp/b.trace && echo "identical"
```

`cmp` with no output is the answer you want. `diff` is fine too, but `cmp` compares bytes,
which is the claim being made.

The fingerprint line is a faster check when you only want to compare against a number
someone else reports:

```bash
./target/release/salman run examples/conveyor/conveyor.st --until T#5s --record Motor,State \
  | grep fingerprint
# fingerprint: ced6dae759c95680c88b846197c8a403e700c7becb16ecf2d5aea877a28dceab
```

With `--trace FILE` the fingerprint appears on the `trace written to …` line instead, so
either form gives you the number to compare.

To compare across machines, run the same command on each and compare that one line. Until
the workflow does it, that is the whole cross-platform check, and it is worth doing by hand
before a release.

`salman run` flags that matter here: `--scans N` or `--until T#5s` to bound the run,
`--record A,B` for the columns, `--trace FILE` to write instead of printing.

**`--record` and a test file's `record:` list do not resolve names the same way**, and the
difference bites when you move a signal from one to the other. `find_signal` in
`crates/salman-cli/src/main.rs` accepts a `%` address, an exact full slot name
(`--record Conveyor.Parts.CV`), or a bare *final* segment (`--record Motor`) — and when the
final segment matches several slots it takes the first with `.position(...)`, silently. A
partial dotted path is refused: `--record Parts.CV` gives *"no variable called Parts.CV"*.
`runner::resolve` in `crates/salman-test/src/runner.rs` matches any dotted *suffix*, so
`Parts.CV` resolves there — which is what the conveyor golden test relies on — and it refuses
an ambiguous name by naming the candidates, narrowed by `pou:` if the case sets one.

### The golden-trace tests

```bash
cargo test --workspace                                        # everything
./target/release/salman test examples/conveyor/conveyor.st examples/conveyor/
```

The second one runs eight declarative tests, the last of which compares a recorded trace
against `examples/conveyor/conveyor.trace`. It takes about 60 ms on an Apple silicon laptop
and covers roughly 35 000 scans of simulated plant time.

A warning about a filter you may be tempted to use: **`cargo test --workspace determinism`
matches nothing at all.** `cargo test` filters on the substring of a test's full path, and no
test in the tree has `determinism` in its name — they all read `..._is_deterministic`.
`cargo test --workspace determinis` does match, and finds nine functions across `clause.rs`,
`capability.rs`, `clock.rs`, `pcap.rs`, `model.rs`, `roundtrip.rs`, `lexer.rs`, `parser.rs`
and `timeline.rs`. That is a useful smoke test and it is not a determinism suite. The golden
traces are what actually check the runtime, and they run under the plain
`cargo test --workspace`.

### Regenerating a golden trace

```bash
./target/release/salman test examples/conveyor/conveyor.st examples/conveyor/ --update-golden
```

Then **read the diff**. A golden trace is a text file precisely so that its diff is
reviewable in a pull request; regenerating one without reading it converts the strongest
test in the repository into a rubber stamp. If the diff is not the change you intended, you
have found a determinism bug, and that is the good outcome.

The same applies to the other golden artefacts, which use an environment variable instead:
`SALMAN_UPDATE_GOLDEN=1 cargo test -p salman-plcopen --test compat` and the analyser's
reports in `crates/salman-analyse/tests/golden.rs`.

## The checklist before you push

If your change touched the runtime, the trace format, or anything ordered:

1. `cargo clippy --workspace --all-targets` — clean. This is where a `HashMap` or a `sin`
   gets caught.
2. `cargo test --workspace` — clean, including the golden traces.
3. Run the same project twice and `cmp` the traces.
4. If you added a trace header field, ask what it is derived from. If the answer involves
   the machine, the clock, the filesystem or the environment, it does not go in the header.
5. If you added a float to the output path, check how it is formatted and confirm the
   fingerprint still comes from `write_canonical_bytes` and not from the rendered text.
6. If you needed a hash map, an allow-at-the-site with a reason, not a wider allow.

## If you change the reference program

`examples/determinism/hazards.st` is a fixture, and the rule for it is the one in its own
header: **every variable earns its place by being a hazard `ADR-0005` names.** Adding a column
because the trace looks short is how a gate turns into decoration.

Two things follow.

**Adding a column changes the fingerprint on every platform at once**, so the gate stays green
and tells you nothing about whether the change was right. Run it locally before and after and
read the diff yourself; the gate cannot do that for you.

**When salman grows a hazard, the fixture grows a column in the same commit.** The ones absent
today are absent from salman, not judged unimportant: transcendentals wait on `libm`, `**` is
refused by the compiler as `U0501`, and there are no `HashMap`s to exercise. When `SIN` lands,
it belongs in this file, and `examples/determinism/README.md` has the table to extend.

## What is still owed

The gate exists; the **claim** does not. `ADR-0005` withholds it "until that job has been green
on all three operating systems for a meaningful period", and one green run is evidence, not a
period. `README.md` rule 6 says the same. Until then the honest sentence is that salman checks
cross-platform determinism on every push and does not yet assert it.

Two gaps worth knowing, neither of which this gate closes:

- **The reference program has no committed golden trace.** A change that altered its trace on
  all three platforms equally would pass. The conveyor's golden covers that for the constructs
  it uses; nothing covers it for the float columns.
- **A toolchain bump invalidates the premise** until the gate has run again, which is why
  `rust-toolchain.toml` calls itself a reviewed change.
