# salman

A vendor-neutral, text-first, git-native workbench for IEC 61131-3 PLC engineering,
industrial communication, and deterministic-network simulation.

**Version 0.0.1. Pre-alpha.** What exists today is the language core: Structured
Text compiles and runs on an embedded deterministic runtime, and a declarative
test suite runs against it headless, on any operating system, with no vendor
licence. There is no user interface, no fieldbus, no network model and no AI
layer; those are the roadmap, not the product. `docs/CONFORMANCE.md` says
exactly what is implemented, what is tested, and what salman had to decide for
itself because no source available to it settled the question.

---

## Sixty seconds

```
$ cargo build --release            # about 20 s from a cold cargo cache
$ target/release/salman check examples/conveyor/conveyor.st
examples/conveyor/conveyor.st: no errors

$ target/release/salman test examples/conveyor/conveyor.st examples/conveyor/
 pass  the belt does not run until the start button is pressed  (2 scans, T#1ms)
 pass  the belt runs on for two seconds after the stop button, then stops  (2003 scans, T#2s2ms)
 pass  the stop button wins when both buttons are pressed, and there is no start-up pulse  (3002 scans, T#3s1ms)
 pass  each part counts exactly once, on the leading edge of the sensor  (9 scans, T#8ms)
 pass  the batch completes at ten parts and progress reaches one hundred percent  (21 scans, T#20ms)
 pass  a jam is flagged after ten seconds of running with no part  (10002 scans, T#10s1ms)
 pass  a part restarts the jam timer  (20002 scans, T#20s1ms)
 pass  the whole sequence produces the recorded trace  (17 scans, T#16ms)

8 tests: 8 passed, 0 failed, 0 errored, 0 skipped
```

Twenty thousand scans of simulated conveyor, twenty seconds of simulated time,
in a few milliseconds of real time — the same answer on every machine, with no
PLC and no licence server. The test file is
[`examples/conveyor/conveyor.salman-test.yaml`](examples/conveyor/conveyor.salman-test.yaml);
the golden trace it compares against is a text file you can read and diff.

There is no screenshot because there is no graphical interface yet. It arrives
with the workbench milestone; see `docs/ROADMAP.md`.

---

## What this is NOT

- **Not a safety PLC and not a safety tool.** Nothing here is IEC 61508 /
  IEC 62061 / ISO 13849 certified. Never use it to design, validate or replace a
  safety function.
- **Not a certified runtime.** The embedded runtime is for development, testing
  and simulation. It is not for controlling machinery, and the docs must say so
  on every page that mentions it.
- **Not a replacement for vendor engineering tools.** Downloading to a physical
  PLC, commissioning, and vendor-specific configuration remain vendor tooling's
  job. `salman` reads, writes, analyses, simulates and tests.
- **Not a 3GPP-compliant 5G stack.** The network layer models the *effect* of a
  network on a control loop, parameterised from published profiles. It does not
  implement RAN or core.
- **Not a plant physics engine.** Process models are simple and pluggable. For
  real physics, it federates with an external simulator; it does not pretend to
  be one.
- **Not an offensive-security tool.** Read-only by default, always.

## Security and safety boundary

`salman` is an **engineering and diagnostic tool**. It is designed for networks
and equipment the user owns or is authorised to work on.

- Read-only by default; writes require an armed posture and per-call
  confirmation.
- No network discovery outside explicitly user-declared address ranges.
- No credential guessing, no exploitation, no fuzzing of live equipment, no
  denial of service, no firmware manipulation. These capabilities are not
  present and will not be added.
- Captures may contain process data and credentials: redaction is on by default
  in exported bundles, and the redaction rules are documented and testable.
- Follow IEC 62443 concepts in the architecture and say which parts of it the
  tool addresses and which it does not.

At 0.0.1 there is no code path in this repository that opens a socket, writes to
a device, or changes a controller mode. The posture model
(`crates/salman-core/src/posture.rs`) exists before it is needed, so that the
first write path cannot be written without going through it. Firmware
operations, credential guessing and denial of service are refused there at every
posture, in code, and are not configuration options. See
[`SECURITY.md`](SECURITY.md) and
[`docs/adr/ADR-0002-read-only-by-default.md`](docs/adr/ADR-0002-read-only-by-default.md).

**Not a safety tool.** salman is not certified, assessed, qualified or approved
under IEC 61508, IEC 62061, ISO 13849 or any other functional safety standard,
and no such assessment is planned. See [`LEGAL.md`](LEGAL.md).

---

## The standard, and which edition

salman targets **IEC 61131-3:2013 (Edition 3.0)**. That edition was **withdrawn
on 2025-05-22** and superseded by IEC 61131-3:2025 (Edition 4.0), which is the
current edition. salman targets Edition 3.0 because it is the edition its public
sources allow it to verify; targeting one it cannot check would be guessing.

Clause numbers are edition-specific — Structured Text is §7.3 in Edition 3.0 and
§7.2 in Edition 4.0 — so salman never writes a clause number without its year and
edition. No normative IEC text is reproduced anywhere in this repository.
[`docs/IEC_CITATIONS.md`](docs/IEC_CITATIONS.md) lists every citation salman
makes and how far each number could be cross-checked against a public source.

**salman claims no conformance and no compliance.** It aims at Structured Text
and publishes a per-feature account in [`docs/CONFORMANCE.md`](docs/CONFORMANCE.md).

---

## The performance budget

Rule 5: the lightweight budget is a tested gate, not a slogan.
`.github/workflows/perf.yml` measures each of these and fails when a measurement
exceeds the threshold in [`perf-budget.toml`](perf-budget.toml).

| Measurement | Budget | Measured |
|---|---|---|
| Cold start (`salman version`, warm page cache, median of 20) | 2 s | **2.8 ms** |
| Release binary on disk | 120 MB* | **2.4 MB** |
| Peak resident set (30 000 scans of the conveyor example) | 350 MB | **2.9 MB** |
| `cargo test --workspace`, excluding the build | 60 s | **0.4 s** |
| Scan of a 1000-rung program | a small fraction of one core | **60 µs, or 0.6 % of one core at a 10 ms period** |

Measured on an Apple M-series laptop with the pinned toolchain. The numbers on a
shared-tenant CI runner are worse and noisier, which is why the committed
thresholds are ceilings rather than targets.

\* The 120 MB figure is an **installer** budget. There is no installer yet, so
that job currently weighs the CLI binary against the same number; `perf-budget.toml`
says so rather than letting a passing run be read as evidence the installer fits.

---

## What actually works

Generated from the capability registry in `crates/salman-core/src/capability.rs`,
which refuses to call anything tested unless it names tests that exist. The full
table is [`docs/STATUS.md`](docs/STATUS.md); `salman status` prints it.

**Working end to end:** Structured Text — lexer, parser, type checker, bytecode
compiler and a deterministic scan runtime. All ten IEC standard function blocks.
Cyclic, event and freewheeling tasks with a correct process image. Declarative
unit tests and golden-trace tests, with JUnit XML output and a real exit code.
Two dialect profiles.

**Not written:** every graphical language, Instruction List, the Edition 3
object-oriented extensions, references, most of the standard function library,
IO mapping for `AT %` located variables, every protocol, every importer, the
network model, the desktop application and the AI layer. Meeting one of these in
source produces a message naming what is missing, not a confusing failure.

---

## Prior art, and what is actually new here

There is a great deal of existing work, and salman is careful not to overclaim.
[IronPLC](https://github.com/ironplc/ironplc) is an actively developed Rust
IEC 61131-3 toolchain with a bytecode VM, ladder support and an MCP server.
[PLC-lang/rusty](https://github.com/PLC-lang/rusty) is a Rust Structured Text
compiler via LLVM. [matiec](https://github.com/beremiz/matiec) and
[Beremiz](https://github.com/beremiz/beremiz) have been doing this for years.
salman claims novelty for **none** of: a Rust ST parser, a Rust ST compiler, an
open-source IEC 61131-3 runtime, PLC static analysis, PLC unit testing, PLC unit
testing in CI, AI or MCP integration with a PLC compiler, or network
co-simulation of control loops. Each of those has a maintained open-source
implementation today. `docs/ROADMAP.md` lists them.

What salman does claim, in the only shapes the evidence supports:

1. **A bounded negative result.** We could not find an open-source tool that
   performs semantic, graph-level diff of graphical IEC 61131-3 logic usable as a
   git diff or merge driver. Commercial tools exist. This is a bounded search
   result, not proof; if you know of prior art, open an issue and we will amend
   this paragraph. *(salman does not implement this yet either — it is the
   workbench milestone.)*
2. **The gap, not the idea.** Every open-source PLC unit-testing framework we
   found requires a proprietary runtime: TwinCAT, CODESYS, Sysmac Studio or TIA
   Portal. PLC unit testing in CI is not new. What is absent is doing it without
   a vendor licence, and that part works today.
3. **Integration stated as integration.** No single tool we found combines
   compiler, deterministic runtime, semantic diff, CI unit tests and network
   co-simulation. Integration is the contribution; salman did not invent the
   parts.

---

## Engineering rules

Hard gates. A pull request violating any of them does not merge.

1. **CI exists before feature #1.** The first commit that adds source also adds
   `.github/workflows/ci.yml`.
2. **Never document a surface that does not exist.** A stub says it is a stub, in
   its own output and in the docs.
3. **One source of version truth** — the root `VERSION` file, checked when
   `salman-core` compiles, so it cannot drift on any machine.
4. **Every URL in every doc resolves**, checked in CI.
5. **The lightweight budget is a tested gate**, not a slogan.
6. **Determinism.** Same project, same inputs, same seed, identical trace, bit
   for bit. See [`docs/adr/ADR-0005-determinism.md`](docs/adr/ADR-0005-determinism.md)
   for the mechanisms and for the honest scope of the claim — cross-platform
   determinism is currently an untested premise that the determinism workflow
   exists to settle.
7. **Untrusted input is treated as hostile.** Every parser is fuzzed in CI and
   must never panic, allocate without bound, or read out of bounds.
8. **Read-only by default.**
9. **Compatibility claims are generated, never written.**

---

## Building

```
cargo build --workspace
cargo test --workspace
```

Requires the toolchain pinned in `rust-toolchain.toml`, which `rustup` installs
automatically. Nothing else: no C compiler, no vendor SDK, no licence server.

## Documentation

| | |
|---|---|
| [`docs/CONFORMANCE.md`](docs/CONFORMANCE.md) | What of IEC 61131-3 is implemented, tested, absent, or a salman decision |
| [`docs/STATUS.md`](docs/STATUS.md) | Generated capability table |
| [`docs/ROADMAP.md`](docs/ROADMAP.md) | Milestones, open questions and prior art |
| [`docs/IEC_CITATIONS.md`](docs/IEC_CITATIONS.md) | Every clause salman cites, and how far each number was verified |
| [`docs/AI_POLICY.md`](docs/AI_POLICY.md) | How salman is developed, and the policy for the AI layer it will ship |
| [`docs/adr/`](docs/adr/) | Architecture decisions, numbered without gaps |
| [`LEGAL.md`](LEGAL.md) | Standards copyright, trademarks, safety, licence, export control |
| [`SECURITY.md`](SECURITY.md) | Reporting, supported versions, what salman deliberately cannot do |

## Licence

Apache-2.0. See [`LICENSE`](LICENSE).

salman is an independent open-source project. It is not affiliated with, endorsed
by, or sponsored by any third party. References to third-party names are
descriptive only; see [`LEGAL.md`](LEGAL.md).
