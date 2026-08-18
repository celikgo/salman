# Roadmap

This document states **intent**. It is not an inventory and it is not a promise of a date.

The inventory is `crates/salman-core/src/capability.rs`, which generates every status table
salman publishes, and `docs/CONFORMANCE.md`, which says feature by feature what the language
front end and the runtime actually do. Where those two disagree with this page, they are
right.

Everything below the v0.2 section is written in the future tense on purpose. None of it
exists. There is no GUI, no AI layer, no importer, no network model, no plant model and no
fieldbus other than Modbus in this repository at 0.0.1. The `salman` binary has five
subcommands — `version`, `status`, `check`, `run` and `test` — and nothing beyond them. Two
v0.2 items are done and are marked so below; the rest of v0.2 is future tense as well.

Status markers are the four from the capability registry — shapes, not colours, because a
red/green table some readers cannot distinguish is a defect:

`[x]` implemented and tested · `[~]` implemented, untested · `[-]` stub · `[ ]` planned

---

## Where v0.1 stands today

The capability registry currently holds twenty-nine entries: twenty-eight `[x]` and one
`[~]`. The workspace test suite passes; at the time of writing `cargo test --workspace` runs
851 tests. Both numbers are a snapshot of a moving tree — the registry, not this page, is the
authority, and `docs/STATUS.md` is generated from it.

The pipeline runs end to end. A Structured Text file is lexed, parsed, checked, compiled to
bytecode and executed on the scan runtime, and `examples/conveyor/` is a worked example with
eight declarative tests, one of them a golden-trace test.

### Claimed, and tested

| Status | Capability |
|---|---|
| `[x]` | The capability registry itself, with tests cited as evidence and checked to exist |
| `[x]` | The IEC clause citation registry, and `docs/IEC_CITATIONS.md` generated from it |
| `[x]` | Deterministic seeded RNG, written in-crate against published test vectors |
| `[x]` | Diagnostics with spans, clause citations and the dialect rule that was applied |
| `[x]` | Case-insensitive, case-preserving IEC identifiers |
| `[x]` | The OBSERVE / SIMULATE / ARMED posture model with categorical refusals |
| `[x]` | Source map, spans and line/column resolution |
| `[x]` | `TIME`, `LTIME`, `DATE`, `TIME_OF_DAY`, `DATE_AND_TIME` values |
| `[x]` | In-crate SHA-256 trace fingerprint with known-answer tests |
| `[x]` | Elementary types, the `ANY` hierarchy and runtime values |
| `[x]` | One source of version truth, checked when the crate compiles |
| `[x]` | The recursive-descent ST parser, with error recovery and bounded nesting |
| `[x]` | Dialects as configuration, with every diagnostic naming the rule it applied |
| `[x]` | The bytecode compiler, with static instance layout and no run-time allocation |
| `[x]` | The bytecode interpreter, which faults rather than panics, with a scan watchdog |
| `[x]` | Scan semantics with a correct process image, and a visible force list |
| `[x]` | `RETAIN` and `PERSISTENT` across simulated warm and cold restarts |
| `[x]` | Cyclic, event and freewheeling tasks with priority and overrun detection |
| `[x]` | All ten IEC standard function blocks, with their awkward edge cases asserted |
| `[x]` | The virtual clock, so a ten-minute sequence tests in milliseconds, identically |
| `[x]` | Declarative unit tests for POUs, on a virtual clock, with no vendor runtime |
| `[x]` | Golden-trace tests against a reviewable text file |
| `[x]` | JUnit XML report and a real exit code, for a build server |
| `[~]` | libFuzzer targets for the front end — they run daily, and finding nothing is not evidence |

### In the tree, working, and deliberately not claimed yet

These have code and passing tests but no entry of their own in the capability registry. Until
a capability is in the registry, salman does not claim it, and this page will not pretend
otherwise:

- the ST lexer (its behaviour is exercised through the parser's entry, but it has no entry of
  its own);
- semantic analysis — name resolution, type checking, constant folding and the static
  recursion check — which is exercised through the compiler's entry and has none of its own;
- the type rules: the implicit conversion table and the operator domains, written as data and
  tested as data;
- the bytecode instruction set itself, as distinct from the compiler and the interpreter that
  are claimed;
- the trace recorder, as distinct from the fingerprint over it, which is claimed;
- `SEMA`, which is shipped, is not an IEC standard function block, and is never described as
  one.

### Not started

- **The project file.** There is no manifest format; the dialect and the source files are
  command-line arguments. `salman test` therefore still takes exactly one source file.
- **The formatter.** There is no `salman fmt`, and nothing renders an AST back to source.
- **Exponentiation.** The compiler reports `**` as not implemented rather than compiling it
  to something approximate.
- **Fuzzing of the declarative test-file reader**, and of every decoder salman later grows.

### What "v0.1" will mean

v0.1 ships when a person can write a Structured Text file, and salman can check it, run it
deterministically, and test it — headless, with no vendor tool anywhere. Most of that now
works:

`[x]` semantic analysis: names, types, constants, and the static recursion check the memory
layout depends on · `[x]` code generation to bytecode · `[x]` `salman check`, `salman run`,
`salman test` · `[x]` the golden-trace harness, with the trace fingerprint as the oracle ·
`[x]` the parser fuzzed as the lexer already is · `[x]` every one of the above in the
capability registry with named tests · `[ ]` a project file · `[ ]` a formatter whose output
is stable · `[ ]` the cross-platform trace comparison in `.github/workflows/determinism.yml`,
which is still a placeholder.

---

## v0.2 — talk to something

The first version that leaves the process. Two items are done; the rest is future tense.

- `[x]` **`AT %` located variables.** A variable declared `AT %QX0.0` **is** that location: it
  has no slot, because a slot would be a copy, and a copy of an input is correct right up
  until the moment it matters. This is the door every protocol added later comes through.
- `[x]` **Several source files as one program.** `salman check` and `salman run` take a list
  of files; a `PROGRAM` in one may call a `FUNCTION_BLOCK` in another. `salman test` still
  takes one, which is a limit the project file removes.
- `[x]` **A Modbus client and a Modbus simulator**, so that a test can drive both ends
  without any hardware. The wire format, both framings, the server's data model, the client
  and the simulator are all in the tree, and every write goes through the posture model.
  Nothing reads or writes a serial port yet.
- **I/O mapping**: binding a device's registers to `%I` and `%Q` in the process image, with
  the mapping declared in the project file rather than in code.
- **Capture, decode and timeline**: recording traffic, decoding it, and putting it on the
  same time axis as the scan trace, because a control problem is almost never visible in
  either alone.
- **PLCopen XML** import and export, with a **compatibility matrix generated by CI** — never
  written by hand — saying what round-trips and what does not.

The posture model already in the tree is what governs this milestone: read-only by default,
writes only from an armed posture and only with per-call confirmation.

## v0.3 — the workbench

- A **Tauri desktop application** over the same headless core, so nothing is reachable only
  through the GUI.
- A **language server** for editors that are not the workbench.
- **Ladder and FBD canvases**, editing the same model the text front end produces.
- **Semantic diff** of graphical logic, and the **GitHub Action** that makes it usable as a
  review tool.
- **Scope and timeline**, **watch and force** — with the force count never hidden, which the
  memory model already enforces.
- **Signed installers**.

## v0.4 — the network

- **OPC UA** client and server.
- **CANopen**, including the CiA 402 drive profile.
- **Channel models** parameterised from **published profiles that are cited**, not invented.
- **TSN schedule analysis**.
- **Co-simulation** with an external simulator, so salman never has to pretend to be a plant
  physics engine.
- A worked **deadline-miss example**: a control loop that misses its deadline because of the
  network, reproducible from a seed.

## v0.5 — the AI layer

Assistance over salman's own artefacts — the AST, the trace, the diff, the capture — with
every action passing through the posture model, and with the same rule as everywhere else:
nothing is claimed that is not tested.

## v0.6 — the field

- **EtherCAT**, including **distributed-clock drift diagnostics**.
- **PROFINET RT**.
- **EtherNet/IP**.
- **S7comm**.
- **MQTT and Sparkplug**.
- **Vendor importers**.
- **5G profiles** for the channel models — the effect of a network on a control loop,
  parameterised from published profiles, never a 3GPP stack.

## v1.0 — freeze

Stable file formats, stable capability identifiers, stable diagnostic codes, and a
compatibility promise that can be tested rather than asserted.

---

## Open questions

Deliberately unanswered. Writing an answer now would be writing a guess.

### Which open EtherCAT and PROFINET stacks to build on

Deferred to v0.6. Licence terms, ETG membership rules and PI conformance costs all change,
and researching them now means researching them again when the work actually starts. The
decision will be made with an ADR at the time, not inherited from a note written years
earlier.

### The copyright holder line

`LICENSE` is Apache-2.0 and the workspace currently attributes authorship to "salman
contributors". Whether that becomes a named person, a company or a foundation is unresolved,
and it has to be resolved before anyone else's contribution is accepted.

### The name and location of the shared AI provider crate

v0.5 needs one crate that owns the provider abstraction, and it is to be shared rather than
duplicated — that much is decided in `docs/adr/ADR-0011-shared-ai-provider-crate.md`. What
it is **called** and **where it lives** are not decided, and that ADR deliberately declines
to invent either. The crate is not created early: salman does not carry empty crates that
imply a surface exists before it does, and the capability registry gets no entry for any of
this until the crate has a name, because a registry entry is a claim about a named thing.

---

## Prior art

There is a great deal of existing work here, some of it excellent, some of it directly
overlapping salman on every axis. This section exists so that nobody reads salman's other
documents and concludes it invented something it did not.

### IronPLC — <https://github.com/ironplc/ironplc>

MIT licensed, very active, and **the most direct overlap on every axis**: an open-source
IEC 61131-3 toolchain written in safe Rust, with a compiler (`ironplcc`), a bytecode runtime
(`ironplcvm`) with task scheduling, a VS Code extension, a browser playground, support for
several source formats including PLCopen XML and TwinCAT projects, and **an MCP server that
lets an AI agent call the compiler**. Rust, Structured Text, a bytecode VM, and AI
integration — salman is doing the same four things.

Its edition-support page,
<https://www.ironplc.com/reference/language/edition-support.html>, is the model
`docs/CONFORMANCE.md` was written against. IronPLC's published language reference also
includes ladder-diagram material; salman could not establish from those pages whether its
compiler implements ladder today, and does not assert either way.

### PLC-lang/rusty — <https://github.com/PLC-lang/rusty>

LGPL-3.0 (the repository also lists GPL-3.0). A Structured Text compiler written in Rust that
uses **LLVM** to produce native code, with a `plc_cfc` crate for the graphical Continuous
Function Chart. Where salman chose a bytecode VM so that it could state exactly what every
operation does, rusty chose a native back end; both are reasonable and they are not the same
project.

### matiec — <https://github.com/beremiz/matiec>

The ST and IL compiler that Beremiz and OpenPLC build on. salman has used it as a **source**,
not merely admired it: two entries in `docs/CONFORMANCE.md` — uppercase-only hexadecimal
digits, and whether a duration literal may carry a sign — exist because matiec takes a
position and cites the standard for it, and vendors do the opposite.

### Beremiz — <https://beremiz.org/> and <https://github.com/beremiz/beremiz>

An integrated IEC 61131-3 development environment with graphical editors, built on matiec. It
has been doing the "open IDE for the whole standard" job for far longer than salman has
existed.

### OpenPLC — <https://autonomylogic.com/> and <https://github.com/thiagoralves/OpenPLC_v3>

An open-source runtime and editor covering all five IEC 61131-3 languages, widely deployed on
real hardware and heavily used in industrial-security research and teaching. Anyone whose
need is "run IEC 61131-3 logic on a small computer today" should look here first.

### PLC unit-testing frameworks

- **TcUnit** — <https://github.com/tcunit/TcUnit>, <https://tcunit.org/> — requires Beckhoff
  TwinCAT 3.
- **coUnit / CfUnit** — <https://github.com/Aliazzzz/coUnit> — a fork of TcUnit; requires
  CODESYS 3.
- **PLCSIM.UnitTest** — <https://github.com/Lorenz-Software/PLCSIM.UnitTest> — requires
  Siemens TIA Portal Openness and PLCSIM Advanced.

The search that produced this list was bounded and is repeatable: it started from the curated
list at <https://github.com/myutzy/awesome-structured-text> and from searches for
vendor-specific test frameworks.

---

## What salman may claim

Three claims, in these shapes and no stronger.

### Claim 1 — a bounded negative result

We could not find an open-source tool that performs semantic, graph-level diff of graphical
IEC 61131-3 logic usable as a git diff or merge driver. Commercial tools exist. This is a
bounded negative search result, not proof; if you know of prior art, open an issue and we will
amend this paragraph.

### Claim 2 — the gap, not the idea

Every open-source PLC unit-testing framework we found requires a proprietary runtime:
TwinCAT, CODESYS, or TIA Portal with PLCSIM Advanced. **PLC unit testing in CI is not new.**
What is absent is doing it without a vendor licence — which is the thing salman is trying to
supply, and it is a gap in availability, not an idea nobody has had.

### Claim 3 — integration stated as integration

We found no single tool that combines a compiler, a deterministic runtime, semantic diff, CI
unit tests and network co-simulation. **The integration is the contribution.** salman did not
invent the parts, and each part has a maintained open-source implementation, several of them
listed above.

## What salman must NOT claim

Each of the following has a verified, currently maintained open-source implementation.
Claiming novelty for any of them would be false:

- a Structured Text parser written in Rust;
- a Structured Text compiler written in Rust;
- an open-source IEC 61131-3 runtime;
- static analysis of PLC programs;
- PLC unit testing;
- AI or MCP integration with a PLC compiler;
- network co-simulation of control loops.

If a salman document, release note, README or talk abstract makes one of those claims, that
document is wrong and this list is the correction.
