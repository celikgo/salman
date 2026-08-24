# salman, for an agent arriving cold

salman is a vendor-neutral IEC 61131-3 workbench: a Structured Text front end, a
deterministic bytecode runtime, a Modbus stack, and a declarative test harness — thirteen
crates of safe Rust, no vendor SDK, no licence server, no GUI.

Most contributors have never read IEC 61131-3, and most agents have never seen a PLC scan
cycle. This file is the orientation. The skills in `.claude/skills/` are the detail.

---

## The house rule

**Never write a claim the code does not support.**

salman talks to industrial equipment. An overstated conformance, protocol or safety claim is
worse than a missing feature, because a missing feature is visible and an overstated one is
not. This is not a style preference — it is enforced in several places, and it is the reason
several documents in this repository read as though they are arguing against themselves.

Its most important consequence has a name and a ritual. **When the standard is ambiguous,
salman does not guess silently.** It picks a policy, marks it `salman policy` in the source,
records it in `docs/CONFORMANCE.md` under `## salman policy`, and gives it a test whose
function name is a sentence. Thirty are recorded there today.
`.claude/skills/extending-structured-text/` describes the ritual; do not add a thirty-first
without it.

Two more things follow, and both bite:

- **A capability is `implemented and tested` only if it names tests that exist.**
  `crates/salman-core/src/capability.rs` is the registry, and
  `every_cited_test_exists_in_the_source_tree` reads the source tree and fails the build if a
  cited test was renamed or deleted. The same rule holds for IEC citations in
  `crates/salman-core/src/clause.rs`.
- **Three documents are generated. Editing them by hand is always wrong** — see below.

---

## Layout

Thirteen crates, layered. Everything depends on `salman-core`; nothing depends on
`salman-cli`.

| Crate | Lines | What it owns that nothing else does |
|---|---|---|
| `salman-core` | 7 360 | Spans, diagnostics, identifiers, values, time, SHA-256, the seeded RNG, the posture model, and the two registries — capability and clause. Zero dependencies. |
| `salman-lang` | 16 430 | Lexer, parser, AST, name resolution and type checking. Decides what is *legal*. |
| `salman-vm` | 8 812 | Bytecode compiler, memory layout, the interpreter, the virtual clock, tasks, the trace. Decides what a legal program *does*. |
| `salman-modbus` | 4 771 | Modbus PDUs, TCP and RTU framing, CRC, the server data model. Opens no socket. |
| `salman-modbus-net` | 1 838 | Blocking-socket client and simulator. The only crate that reaches a network. |
| `salman-capture` | 3 823 | pcap read/write, Ethernet/VLAN/IPv4/IPv6/TCP decode, TCP reassembly. Protocol-agnostic. |
| `salman-findings` | 1 297 | What a finding is: its group, its confidence, and the evidence it carries. |
| `salman-analyse` | 2 015 | Turning reassembled bytes into findings, and merging a capture with a scan trace onto one timeline. The Modbus-specific half is `modbus.rs`. |
| `salman-project` | 1 545 | The project file: sources, devices, and the register-to-process-image mapping. |
| `salman-link` | 1 011 | Running those mappings at the scan boundaries, against a `Peer` that is `Simulated` or `Live`. |
| `salman-plcopen` | 2 892 | PLCopen XML read and write, and the round-tripped compatibility matrix. |
| `salman-test` | 1 420 | The `.salman-test.yaml` format, the runner, JUnit XML. |
| `salman-cli` | 4 715 | The `salman` binary. Every capability is reachable headless; that is the point. |

Four direct third-party dependencies in the whole workspace: `clap`, `serde`,
`serde-saphyr`, `xml`. `salman-core`, `salman-lang`, `salman-vm`, `salman-modbus`,
`salman-modbus-net`, `salman-capture`, `salman-findings`, `salman-analyse` and `salman-link`
have none at all.

`salman-plcopen` is not a dependency of `salman-cli`: PLCopen XML is a library surface, not a
subcommand.

---

## Building and testing

```bash
cargo test --workspace          # 1200 tests, about two seconds after the build
cargo fmt --all
cargo clippy --workspace --all-targets
```

CI runs clippy with `-D warnings`, so a warning is a failure.

The toolchain is pinned in `rust-toolchain.toml` to **1.94.1** with `rustfmt` and `clippy`,
`profile = "minimal"`. **Do not change it.** The comment in that file explains why it is a
reviewed change and not a drive-by: the determinism gate is only meaningful if every machine
runs the same compiler.

Edition 2024, `rust-version = "1.94"`, `resolver = "3"`.

### The lints that will bite you

From `[workspace.lints]` in `Cargo.toml`:

- `unsafe_code = "forbid"` — there is none, and adding some needs an ADR.
- `missing_docs = "warn"`, `unreachable_pub = "warn"`, `missing_debug_implementations = "warn"`.
- **`unwrap_used`, `expect_used`, `panic`, `todo`, `unimplemented`, `dbg_macro` are `deny`**
  in library code, and test code re-allows them. Five crates do it crate-wide with
  `#![cfg_attr(test, allow(...))]` at the top of `lib.rs` (or `main.rs`) — `salman-core`,
  `salman-lang`, `salman-vm`, `salman-test`, `salman-cli`. The rest re-allow per file: a bare
  `#![allow(...)]` at the top of each `tests/*.rs`, or `#[allow(...)]` on an inline
  `mod tests`. `salman-lang` additionally denies `clippy::indexing_slicing` outside tests,
  because it slices buffers whose bounds come from untrusted source text.
- `clippy::all` and `clippy::pedantic` at `warn`.

From `clippy.toml`:

- **`std::collections::HashMap` and `HashSet` are `disallowed-types`.** Use `BTreeMap` /
  `BTreeSet`. The tree holds zero `HashMap<` / `HashSet<` type annotations against 23
  `BTreeMap<` / `BTreeSet<`; the single `HashSet` construction anywhere is inside a test in
  `crates/salman-core/src/ident.rs` that exists to exercise a `Hash` implementation.
- **Every `f32`/`f64` transcendental is a `disallowed-method`** — `sin`, `cos`, `tan`, the
  inverse trigonometric functions, `exp`, `exp2`, `ln`, `log`, `log2`, `log10`, `powf`,
  `powi`, `cbrt`, `hypot`. Each carries its reason. Both files' reasons are worth reading
  before you argue with them.

---

## Generated files — never edit these by hand

| File | Generated from | Regenerate with | Drift caught by |
|---|---|---|---|
| [`docs/STATUS.md`](docs/STATUS.md) | `crates/salman-core/src/capability.rs` | `salman status --markdown > docs/STATUS.md` | `capability.rs`, `the_committed_status_document_matches_what_the_registry_renders` |
| [`docs/IEC_CITATIONS.md`](docs/IEC_CITATIONS.md) | `crates/salman-core/src/clause.rs` | `SALMAN_UPDATE_GOLDEN=1 cargo test -p salman-core the_committed_citation_document_matches_what_the_registry_renders` | `clause.rs`, the same test |
| [`docs/PLCOPEN_COMPATIBILITY.md`](docs/PLCOPEN_COMPATIBILITY.md) | round-tripping every construct | `SALMAN_UPDATE_GOLDEN=1 cargo test -p salman-plcopen --test compat` | `compat.rs`, `the_committed_matrix_matches_what_the_code_does_now` |

`docs/CONFORMANCE.md` is **not** generated. It says so about itself, and calls itself the
weakest link in the chain. It is still the authority on what the language front end does,
and it is where a `salman policy` decision has to be recorded.

Golden artefacts — `examples/conveyor/conveyor.trace`, the analyser's reports — are
regenerated with `SALMAN_UPDATE_GOLDEN=1` or `salman test --update-golden`. Read the diff
before committing it; that is the whole reason the trace is a text file.

---

## The CLI

Seven subcommands. There is no formatter, no language server, no debugger, no GUI.

```
salman version
salman status [--markdown]
salman check   <paths...>              [--dialect generic|iec61131-3:2013-strict]
salman run     <paths...>              [--scans N] [--until T#5s] [--record A,B] [--trace FILE] [--dialect …]
salman test    <source> <tests>        [--junit FILE] [--update-golden] [--dialect …]
salman capture <file.pcap>             [--modbus-port N] [--verbose]
salman project <file.yaml>
```

`check` and `run` take several source files and build them as one program. `test` takes
exactly one source file and one test file or directory. Exit codes: `0` fine, `1` the thing
you asked about is wrong, `2` salman could not do the job.

One asymmetry worth knowing before it confuses you: **the CLI and the test harness resolve
variable names differently.** `salman run --record` (`find_signal` in `main.rs`) accepts a `%`
address, an exact full slot name (`Conveyor.Parts.CV`), or a bare *final* segment (`Motor`) —
and when that final segment matches several slots it takes the first, silently. A partial
dotted path such as `Parts.CV` is refused. A `.salman-test.yaml` uses `runner::resolve`
instead, which matches any dotted *suffix* — so `Parts.CV` resolves, as the conveyor's golden
test relies on — and which refuses an ambiguous name by listing the candidates, narrowed by
`pou:` if one is set. Two resolvers; the test harness has the safer one.

---

## The nine engineering rules, and what actually enforces each

From `README.md`. The right-hand column is the part that matters when you are deciding
whether something is safe to change.

| # | Rule | Enforced by |
|---|---|---|
| 1 | CI exists before feature #1 | history; nothing checks it now |
| 2 | Never document a surface that does not exist | the capability registry's evidence test; otherwise review |
| 3 | One source of version truth | `crates/salman-core/VERSION`, checked when `salman-core` compiles, plus `version-consistency.yml` |
| 4 | Every URL in every doc resolves | `docs-links.yml` — lychee over `./**/*.md` **and `./**/*.rs`** |
| 5 | The lightweight budget is a tested gate | `perf.yml` against `perf-budget.toml` |
| 6 | Determinism, bit for bit | partly. See below — this is the one to be careful about |
| 7 | Untrusted input is treated as hostile | `fuzz.yml` (daily, not per-PR), the `deny`-level panic lints |
| 8 | Read-only by default | `crates/salman-core/src/posture.rs`, in types |
| 9 | Compatibility claims are generated, never written | the three drift tests above |

**Rule 6 is the one an agent will overstate — in either direction.**
`.github/workflows/determinism.yml` runs `cargo test --workspace --all-features` on Linux,
macOS and Windows, then prints a warning saying it compared no trace. That warning is about
one placeholder step, and `docs/ROADMAP.md` and `ADR-0005` both record that the designed
mechanism — per-OS trace artefacts compared against each other — is still owed. But the suite
that ran beside it contains `the_recorded_trace_matches_the_committed_golden_file`, which
compares a freshly rendered trace against the committed `examples/conveyor/conveyor.trace` on
each of the three platforms. So agreement with **one** golden trace is gated; the general
cross-platform claim is not. Say that, not less and not more.
`.claude/skills/preserving-determinism/` has the detail.

**Rule 4 covers Rust doc comments too.** A dead URL in a `//!` block fails the build, and
lychee reads `.claude/skills/*/SKILL.md` and this file as well. Prefer backticked paths to
Markdown links inside skills.

---

## Workflows

Nine files. Seven run on every pull request: `ci`, `determinism`, `docs-links`, `interop`,
`perf`, `supply-chain`, `version-consistency`. `fuzz` runs on a daily cron at 03:27 UTC and
on dispatch — **not** on pull requests. `release` runs on a `v*` tag and on dispatch.

`interop` is the one that cannot be replaced by more of salman's own tests: it runs salman's
Modbus against **pymodbus** in both roles. Everything else in this repository checks salman
against salman, which cannot find the class of error where a decoder and an encoder agree
about the same misreading.

---

## Conventions

- **Every `.rs` file starts with `// SPDX-License-Identifier: Apache-2.0`.** New files too.
- **Test function names are sentences.** `a_call_with_enable_false_does_not_happen_at_all`,
  `nan_is_canonicalised_so_traces_cannot_differ_between_architectures`,
  `the_recorded_trace_matches_the_committed_golden_file`. A test name is read in a failure
  report by someone who does not have the file open; make it say what broke.
- **Module docs argue.** A `//!` block here explains why the module is shaped the way it is
  and what was rejected, often at length, with citations. Match that. A comment that repeats
  the code is noise; a comment that records a decision is the point.
- **Commits explain why, and name what was rejected.** Use conventional-commit prefixes and
  a real body. `git log` is the model.
- Diagnostic codes are stable for ever once published. See below.

---

## Traps

Ordered by how often they catch people.

1. **Editing a generated file.** The drift test fails with a message naming the regeneration
   command. Read it rather than editing the file back.
2. **Adding a capability or citation entry without a test that exists.**
   `every_cited_test_exists_in_the_source_tree` greps for `fn <name>(` in the file you named.
   Renaming a test therefore breaks a registry entry in a different crate.
3. **`unwrap()` in library code.** It is `deny`, not `warn`. Return a typed error.
4. **Reaching for `HashMap`.** Denied by name in `clippy.toml`. Use `BTreeMap`.
5. **Adding a diagnostic code without checking both files.** Codes live in two places, by
   band: `crates/salman-lang/src/codes.rs` owns `E01xx` lexical, `E02xx` syntactic, `E03xx`
   declarations and symbols, `E04xx` types, the matching `U01xx`–`U03xx`, and `W0xxx`;
   `crates/salman-vm/src/compile.rs` owns the `x05xx` band — `E0501`–`E0504` and `U0501`.
   `diagnostic_codes_are_unique` checks `codes.rs` against itself and cannot see another
   crate, so the workspace-wide check reads the source:
   `no_diagnostic_code_means_two_things_in_this_workspace` in
   `crates/salman-core/src/diag.rs` scans every `pub const … DiagCode("…")` under
   `crates/*/src/` and fails if one number carries two names. It exists because `U0301` meant
   two things up to 0.1.0. A code is published: once it means something, it keeps meaning
   that.
6. **Putting a decision in the wrong crate.** "Is this legal?" belongs in `salman-lang`.
   "What does it do?" belongs in `salman-vm`. A value's representation, a span, a diagnostic
   or a hash belongs in `salman-core`. If a change to the type checker needs a new opcode,
   you have crossed a seam and both crates need a test.
7. **Touching `rust-toolchain.toml`.** Don't.
8. **Widening a claim in prose.** "salman is IEC 61131-3 compliant" is a phrase `LEGAL.md`
   forbids outright. salman *aims at* Structured Text of one named edition and publishes a
   per-feature account. The same care applies to novelty: `docs/ROADMAP.md` has a list of
   things salman must never claim to have invented.

---

## Where to read next

| | |
|---|---|
| `.claude/skills/extending-structured-text/` | Adding a language feature, end to end, and the ambiguity ritual |
| `.claude/skills/preserving-determinism/` | What is actually gated, what is not, and how to check locally |
| `.claude/skills/adding-a-fieldbus-protocol/` | The seam a second protocol sits on, written before one exists |
| `.claude/skills/citing-the-standard/` | The citation policy, which is a legal constraint |
| `.claude/skills/releasing-salman/` | Workflow interactions and the maintainer's manual checks |
| [`docs/PIPELINE_WALKTHROUGH.md`](docs/PIPELINE_WALKTHROUGH.md) | One file from source to passing test, with the real output at each stage |
| [`docs/CONFORMANCE.md`](docs/CONFORMANCE.md) | What of IEC 61131-3 is implemented, tested, absent, or a salman decision |
| [`docs/adr/`](docs/adr/) | Sixteen architecture decisions, numbered without gaps |
| [`CONTRIBUTING.md`](CONTRIBUTING.md) | Test tiers, the ADR process, the eight-point bar for a new protocol |

**salman is not a safety tool.** It is not certified, assessed or qualified under IEC 61508,
IEC 62061, ISO 13849 or anything else, and no such assessment is planned. Nothing here may
be used to design, validate or replace a safety function. See [`LEGAL.md`](LEGAL.md).
