# Contributing to salman

Thank you for looking. This file says how to build salman, how to run each tier
of its tests, how decisions get recorded, and what a new protocol has to clear
before it goes in.

Read [`docs/AI_POLICY.md`](docs/AI_POLICY.md) before opening a pull request:
salman has a stated policy on how it is developed, and it applies to
contributions.

---

## If you have never written Structured Text

Most people arriving here have not, and nothing about IEC 61131-3 is guessable
from a background in general-purpose programming. A PLC program has no `main`,
runs its whole body again every few milliseconds for ever, and reads its inputs
from a snapshot taken before it started. Reading the code without knowing that
will mislead you.

Start with **[`docs/PIPELINE_WALKTHROUGH.md`](docs/PIPELINE_WALKTHROUGH.md)**. It
takes `examples/conveyor/conveyor.st` — a conveyor with a start-stop station, a
part counter and a jam timer — and follows it from the source text through every
stage of the compiler and runtime to a passing test, with the real output at each
step. It explains the scan cycle, what a function block is, and what `RS`, `TOF`,
`TON`, `R_TRIG` and `CTU` do, on the way. It also says which of the thirteen
crates did what, which is the fastest way to learn the layout.

Three commands, and you have run the whole thing:

```bash
cargo build --release
./target/release/salman check examples/conveyor/conveyor.st
./target/release/salman test  examples/conveyor/conveyor.st examples/conveyor/
```

Then break something on purpose — change `T#2s` to `T#3s` in `conveyor.st` and
run the tests again. Two should fail, and the messages should tell you exactly
which timing boundary moved. That is the loop this whole project is built around.

## Where the working knowledge lives

[`CLAUDE.md`](CLAUDE.md) is the one-page orientation: the crate layout, the
toolchain pin, the lints that will bite you, which files are generated, and the
traps. It is written for an agent and works just as well for a person.

The five skills in [`.claude/skills/`](.claude/skills/) are the detail, each on
one job:

| | |
|---|---|
| `extending-structured-text` | Adding a language feature end to end, and the ritual below |
| `preserving-determinism` | What is gated, what is not, and how to check locally |
| `adding-a-fieldbus-protocol` | The seam a second protocol sits on — read before proposing one |
| `citing-the-standard` | The citation policy, which is a legal constraint |
| `releasing-salman` | Workflow interactions and the maintainer's manual checks |

They are plain Markdown. Nothing needs an agent to read them.

## When the standard does not answer the question

This will happen to you, and it is the part of contributing here that has no
equivalent elsewhere. IEC 61131-3 is paywalled, several of its rules are
ambiguous, and vendors resolve them differently. salman's rule is that **it does
not guess silently**. When you hit one:

1. **Mark it in the source**, at the decision, with a comment containing the
   words `salman policy`, saying what the question is, what salman does, and what
   you could not verify or which two sources contradict each other.
2. **Record it in [`docs/CONFORMANCE.md`](docs/CONFORMANCE.md)** under
   `## salman policy`, as the next numbered entry. Thirty are there already; copy
   their shape.
3. **Give it a test whose name is a sentence**, and name that test in the
   `CONFORMANCE.md` entry — for example
   `negative_durations_are_accepted_by_the_generic_dialect_and_refused_by_the_strict_one`.

If you have no choice to defend — you believe something but could not confirm it —
it belongs in that file's `## UNVERIFIED` section instead, with a line saying what
would settle it. The difference matters, and the file explains why.

All three steps in the same pull request. A decision made in code and nowhere
else is the one thing this project treats as worse than a missing feature.

---

## The one rule that matters

**Never write a claim the code does not support.**

salman talks to industrial equipment. An overstated conformance, protocol or
safety claim is worse than a missing feature, because a missing feature is
visible and an overstated one is not. In practice:

- A capability is `implemented and tested` only if it names tests that exist.
  `crates/salman-core/src/capability.rs` holds the registry and a test asserts
  every cited test is really in the tree.
- An IEC citation is only registered if it names tests that exist.
  `every_citation_names_at_least_one_test` and
  `every_cited_test_exists_in_the_source_tree` in
  `crates/salman-core/src/clause.rs` enforce it. A clause salman no longer
  checks is deleted from the registry, not left standing with nothing behind it.
- `docs/STATUS.md`, `docs/IEC_CITATIONS.md` and
  `docs/PLCOPEN_COMPATIBILITY.md` are **generated**. Edit the registry that
  produces them, never the file; a test fails if the committed copy has drifted,
  and its failure message names the command that regenerates it. `CLAUDE.md`
  lists all three commands in one table.
- salman claims no conformance and no certification to anything. See
  [`LEGAL.md`](LEGAL.md) §2 for the phrasings that are allowed and the ones that
  are not.

If a pull request makes a claim, the review will ask which test backs it. That
is not obstruction — it is the whole engineering position of the project.

---

## Building

```bash
git clone https://github.com/celikgo/salman.git
cd salman
cargo build --workspace
```

The toolchain is pinned in [`rust-toolchain.toml`](rust-toolchain.toml) and
`rustup` installs it automatically. Nothing else is needed: no C compiler, no
vendor SDK, no licence server. The workspace forbids `unsafe`.

---

## The test tiers

Run the first three before you push. The rest run in CI, and you can run them
locally if you are touching the area they cover.

### 1. The suite — every pull request

```bash
cargo test --workspace
```

1200 tests at 0.1.0, about two seconds after the build. If this is slow,
something is wrong.

### 2. Format and lint — every pull request

```bash
cargo fmt --all
cargo clippy --workspace --all-targets
```

CI runs `cargo fmt --all -- --check` and fails on any diff, and clippy is
configured in [`clippy.toml`](clippy.toml). Both must be clean.

### 3. Determinism — if you touched the runtime, the trace format, or anything ordered

Run the same project twice and compare the bytes:

```bash
cargo build --release
./target/release/salman run examples/conveyor/conveyor.st --until T#5s --record Motor,State --trace /tmp/a.trace
./target/release/salman run examples/conveyor/conveyor.st --until T#5s --record Motor,State --trace /tmp/b.trace
cmp /tmp/a.trace /tmp/b.trace
```

Anything that iterates a `HashMap`, reads the wall clock, or depends on address
layout will break that, which is the point. `cargo test --workspace` covers the
committed golden trace through
`the_recorded_trace_matches_the_committed_golden_file`, and the `determinism`
workflow runs that suite on Linux, macOS and Windows — so a platform that
disagreed with the golden file would fail there. That workflow also records
`examples/determinism/hazards.st` on each platform and compares the three trace
fingerprints against each other, which is the check that says *which* platform
disagreed. See
[`docs/adr/ADR-0005-determinism.md`](docs/adr/ADR-0005-determinism.md) and
`.claude/skills/preserving-determinism/`.

### 4. Fuzzing — if you touched a parser or a decoder

```bash
cargo +nightly fuzz run <target> -- -max_total_time=60
```

Targets live in `fuzz/fuzz_targets`. Every parser salman has is fuzzed, and a
target asserts its postconditions rather than merely that nothing panicked —
spans stay inside the source, node ids stay usable as indices, nothing read goes
missing. Untrusted input is treated as hostile; a parser may never panic,
allocate without bound, or read out of bounds.

### 5. Interop — if you touched Modbus

The `interop` workflow runs salman against **pymodbus** in both roles:
pymodbus's client against salman's simulator, and salman's client against
pymodbus's server. An independent implementation disagreeing with salman is
the only evidence that salman's reading of a wire format is right rather than
merely self-consistent.

### 6. Supply chain

```bash
cargo deny check
```

Advisories, licences, bans and sources, configured in
[`deny.toml`](deny.toml). The licence list is an allowlist: a licence the
project has not considered fails the build rather than entering quietly. Adding
a dependency means adding a row to [`LEGAL.md`](LEGAL.md) §8 as well.

### 7. Performance budget

[`perf-budget.toml`](perf-budget.toml) holds per-platform ceilings for cold
start, binary size, peak resident set and suite duration. Raising one is a
reviewed change to that file — which is the point: the budget can only move in
a commit that says it is moving.

---

## Architecture decisions

Anything that would be expensive to reverse gets an ADR in
[`docs/adr/`](docs/adr/), **numbered without gaps**. Fifteen exist at 0.1.0.

Write one when you are about to decide something a future contributor would
otherwise have to re-derive from the code: a dependency that is hard to remove,
a wire-format interpretation, a determinism-affecting mechanism, a scope
boundary. Copy the shape of an existing one — context, decision, consequences,
and, importantly, **what was rejected and why**.

ADRs are historical records. When reality moves on, write a new ADR that
supersedes the old one; do not rewrite the old one to look correct in
hindsight. That is why several ADRs still say "at version 0.0.1" — that is when
they were written, and the timestamp is the useful part.

---

## The bar for a new protocol plugin

Modbus is the only protocol in the tree, and it took `salman-modbus`,
`salman-modbus-net` and an interop workflow to get there. A second one is
welcome, and it has to clear the same bar:

1. **An ADR first**, covering the addressing model, the endianness decisions,
   and what the specification leaves ambiguous. Ambiguity is normal; deciding it
   silently is not. See
   [`ADR-0012-modbus-addressing.md`](docs/adr/ADR-0012-modbus-addressing.md).
2. **No vendored third-party stack.** salman writes its own decoders. It buys a
   fuzz target salman owns and no `unsafe` between a socket and a decoded frame,
   and it keeps the licence position simple — see [`LEGAL.md`](LEGAL.md) §8,
   including the rule that a GPL/LGPL implementation may be used as an external
   differential-testing oracle run as a subprocess, never linked.
3. **Framing and decoding tested against real captures**, not only against
   salman's own encoder. A decoder checked only against its matching encoder
   tests that the pair agree, not that either is right.
4. **An interop job** against an independent implementation, in both roles where
   the protocol has two. If no such implementation exists, say so in the ADR;
   that is a real constraint and it should be recorded rather than skipped.
5. **A fuzz target** for every decoder, asserting postconditions.
6. **Every write behind the posture model.** Reads need no permission. Writes
   take `Effect::WriteLiveDevice` through `PostureState::permits` and a
   `UserConfirmation` **by value**, so one confirmation authorises exactly one
   write. Firmware operations, credential guessing and denial of service are
   refused at every posture, in code, and are not configuration options. See
   [`SECURITY.md`](SECURITY.md) and
   [`ADR-0002-read-only-by-default.md`](docs/adr/ADR-0002-read-only-by-default.md).
7. **A trademark row** in [`LEGAL.md`](LEGAL.md) §5, and a plain statement that
   salman is not certified by and not affiliated with the owning organisation.
   Speaking a protocol and being certified against it are different things.
8. **Capability registry entries** naming the tests, so the protocol appears in
   `docs/STATUS.md` at the status its evidence supports and no higher.

---

## Pull requests

- Branch from `main`. Keep the change focused.
- The commit message should say *why*, not only *what*. The existing history is
  the model: it explains the reasoning, and it names what was rejected.
- All eight workflows must be green: `ci`, `determinism`, `docs-links`, `fuzz`,
  `perf`, `supply-chain`, `version-consistency`, `interop`.
- Every URL you add must resolve — `docs-links` checks them.
- If you change the version, change it in [`crates/salman-core/VERSION`](crates/salman-core/VERSION) only. It is the
  one source of truth; `salman-core` fails to compile if Cargo disagrees with
  it, and `version-consistency` checks the rest of the metadata and the output
  of `salman version`. See
  [`ADR-0008-one-version-truth.md`](docs/adr/ADR-0008-one-version-truth.md).

## Reporting a security issue

Do not open a public issue. See [`SECURITY.md`](SECURITY.md), which is honest
about the fact that salman has no published security contact address yet and
says what to do instead.

## Conduct

[`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md) applies to every space this project
uses.

## Licence

Contributions are accepted under Apache-2.0, the licence of the project. Every
source file carries an `SPDX-License-Identifier: Apache-2.0` line; new files
must too.
