# ADR-0016: The next thing salman builds is a way to watch a program run, not a second protocol

- **Status**: Accepted (not yet implemented)
- **Date**: 2026-08-24
- **Deciders**: the salman authors

## Context

At 0.1.0 salman can check a Structured Text file, run it deterministically, and test it
headless with no vendor licence. That was the whole of v0.1 and it works. `README.md` states
what is missing in one sentence: *"There is no user interface, no protocol but
Modbus, no network model and no AI layer; those are the roadmap, not the product."*

Two of those four are candidates for the next piece of work, and they pull in opposite
directions. A second fieldbus makes salman reach further into a plant. A viewport makes salman
usable by a person who is already using it. This decision records which one is next, and why,
because the answer was not obvious and the cost of getting it wrong is a year.

### What the roadmap already says

`docs/ROADMAP.md` places **the workbench at v0.3** — a desktop application, a language server,
ladder and FBD canvases, semantic diff, and *"**Scope and timeline**, **watch and force** —
with the force count never hidden, which the memory model already enforces"*. It places **the
second protocol at v0.4** — OPC UA and CANopen — and the rest of the fieldbus zoo at v0.6.

So the roadmap already answers this question, in favour of the viewport. This ADR exists
because a roadmap states intent, not evidence, and because the ordering deserved to be checked
against the code before another milestone was built on it. It was checked. The ordering holds,
and the reason it holds is stronger than the roadmap says.

### The thing a user cannot do today

A person can write a program, check it, run it and test it. **They cannot look at it.** There
is no way, from any interface salman ships, to see a variable change while a program runs.

`salman run` executes every scan and then prints one finished trace. `salman test` reports
pass or fail against expectations written in advance. Neither lets a person ask the question an
engineer asks first at a machine — *what is that value doing right now* — and neither lets them
change an input and watch what happens.

The engineer salman is for works in a vendor IDE, and the first thing a vendor IDE does is put
a variable's live value on the screen. salman has every part needed to do the same and assembles
none of them.

### What is already built

This is the decisive evidence, and it is worth being specific, because "we nearly have it" is
the kind of claim this project does not accept without naming the functions.

Stepping a program one scan at a time: `Runtime::step(&mut self) -> StepOutcome`
(`crates/salman-vm/src/task.rs`), with `run_scans` and `run_until` beside it. One scan is latch,
execute, publish.

Reading a variable by name: `Program::slot_index(&self, name: &str) -> Option<SlotId>`, which
is case-insensitive because IEC identifiers are, then
`Memory::read_slot(&self, slot) -> Option<&Value>`, which honours any force on the slot. That
pair is already the watch-window harness in `crates/salman-cli/tests/semantics.rs`, whose
module header says the tests *"are written against behaviour an engineer can see in a watch
window"* and that *"Reading a slot by its dotted name is what a watch list does"*.

A symbol table: `Program::slot_names: Vec<String>`, doc-commented *"The name of every slot, for
traces and the watch list"*, with `slot_types` beside it.

Driving and forcing: `Memory::write_slot`, `drive_input`, `force`, `release`, `release_all`,
`forces()`, `force_count()` — that last doc-commented *"Never hidden from an interface"* — and
`read_slot_unforced`, doc-commented *"This is what a debugger shows next to the forced value"*.
`Runtime::memory_mut()` exists and is documented *"for driving inputs and setting forces"*.

A working precedent for all of it: `crates/salman-test/src/runner.rs` already does set, force,
release, advance and scan, by name, driven from a YAML file.

**And here is the sentence that decided this ADR.** Grep the workspace for callers of
`forces()`, `force_count()` and `read_slot_unforced`, and there are none outside
`crates/salman-vm/src/memory.rs` and its own test module. The memory model was built to make a
force visible to an interface, and that decision was tested —
`a_force_records_what_the_program_wanted_so_the_difference_is_visible` — but **no interface has
ever collected on it.** salman has kept a promise nobody can see it keep.

### What a second protocol would cost

Measured, not estimated. Modbus as it stands in the tree — both crates, their tests and examples,
their three fuzz targets, the interop workflow and its Python harness, and `ADR-0012` — is
**7 242 lines**, carrying **133 tests**, about 11% of the 1 199 `#[test]` markers in the
workspace. It arrived in about **8 000 insertions** across seven commits, from
*"v0.2: the Modbus wire format, with no transport attached"* to
*"Check salman's Modbus against an implementation nobody here wrote"* — the exact total
depends on which of the adjacent project-file and IO-mapping commits you count as Modbus's.

Modbus is one of the simplest fieldbus protocols in existence. OPC UA is not.

The structural costs are larger than the line count and nobody has priced them:

- **There is exactly one trait in the entire workspace** — `ConfirmationPrompt` in
  `crates/salman-core/src/posture.rs`. There is no device, transport or protocol abstraction of
  any kind. A second protocol does not slot into a seam; it creates one.
- **The project file format is Modbus-shaped.** `crates/salman-project/src/spec.rs` imports
  `salman_modbus::device::Table`, `Protocol` has exactly one variant, and `TableName` hard-codes
  the four Modbus tables into the YAML schema. That schema is a compatibility surface.
- **`salman-link` is hard-wired.** `Link::new` takes a concrete `salman_modbus_net::Client`.
- **`ADR-0013` may be tripped.** It says blocking sockets suffice and names what would change
  that: *"a client that keeps many connections open at once, a simulator that has to hold
  hundreds, or serial RTU"*. OPC UA plausibly qualifies, and an async runtime is a superseding
  ADR and several new dependencies.
- `ADR-0014` stands regardless, so a second protocol buys **read-only** field access. It cannot
  close a control loop, and that is deliberate and permanent.

None of that is an argument against a second protocol. It is an argument that a second protocol
is a milestone, not a next step, and that it should be attempted with the workspace generalised
first rather than during.

## Decision

**The next capability salman builds is `salman monitor`: a headless viewport over a program
running on the simulation runtime. The second fieldbus protocol stays at v0.4, where the
roadmap already put it.**

Scoped deliberately small, and stated so that it can be quoted:

1. **Step and run.** Advance one scan, N scans, or until a simulated instant, and render after
   each stop. `Runtime::step` and `run_scans` already do the work.
2. **A watch list.** Named variables, rendered with their values after every stop, using
   `Program::slot_index` and `Memory::read_slot`. Located variables are named by address,
   because a variable declared `AT %` has no slot — which is already how `Signal` distinguishes
   them.
3. **Set, force and release, by name.** Through `Memory::write_slot`, `force` and `release`,
   with `drive_input` for a located variable, which takes a `DirectAddress` rather than a name.
   `crates/salman-test/src/runner.rs` already drives exactly this set.
4. **A forced value never rendered alone.** A forced slot shows the force *and* what the program
   last tried to write, from `read_slot_unforced`, and **the force count is always on screen**,
   from `force_count()`. This is the promise the memory model has been keeping unobserved, and
   collecting on it is half the point of this work.
5. **One name resolver, and it is the one that refuses.** `salman-test`'s private
   `resolve` collects every candidate and returns an error — *"`{name}` is ambiguous: it could be
   {…}. Write the full name, or set `pou:`"* — while the CLI's `find_signal` takes the first
   suffix match with `.position(...)` and says nothing. A monitor that silently watches the wrong
   `Running` when two POUs have one is worse than no monitor. The refusing resolver is promoted
   to a shared home and both call it.

What this decision deliberately **excludes**:

- **No graphical interface.** The Tauri application remains v0.3, over the same headless core.
- **No new dependency, and specifically no terminal-UI crate.** `salman-cli`'s only third-party
  dependency is `clap`, the workspace has four in total, and a TUI crate means a `LEGAL.md` §8
  dependency row — and a `deny.toml` licence-allowlist entry if its licence is not already on
  the list — for a rendering convenience. Line-oriented output that a person can read and a
  pipe can consume is enough, and it keeps the monitor scriptable.
- **No breakpoints, no watchpoints, no single-stepping inside a scan.** A scan is atomic —
  `docs/CONFORMANCE.md` policy 17 — and stopping inside one would mean deciding what a
  half-executed scan means. It does not mean anything on a controller.
- **Nothing against a live device.** `ADR-0014` is untouched: output mappings run against a
  simulated device only, and there is no posture, flag or configuration key that changes it.

## Consequences

**salman becomes usable for the thing it is already good at.** The gap this closes is not a
missing feature, it is a missing surface onto features that exist and are tested. A person
writing a jam timer can watch the timer, poke the sensor, and see the lamp — which is the loop
every control engineer works in, and the loop salman currently cannot offer at all.

**The v0.3 workbench gets its core first, and gets it tested headless.** Scope, watch and force
are the whole of one of the six v0.3 bullets. Building them behind a command line means the desktop
application inherits a tested core rather than growing one behind a GUI where nothing in CI can
reach it — which is `ADR-0009`'s position applied to a milestone rather than to a repository.

**A name-resolution defect gets fixed rather than shipped twice.** The CLI's silent first-match
resolver is a real bug today: `salman run --record Running` resolves by last dotted segment and
takes whichever matching slot comes first, with no diagnostic. It is invisible now because a
trace column is easy to sanity-check by eye. It would not stay invisible in a monitor.

**The second protocol arrives into a better-shaped workspace, later.** Whoever writes it will
still have to generalise `salman-project`'s schema and `salman-link`'s client, and this decision
does not do that work — but it does not add to it either, and
`.claude/skills/adding-a-fieldbus-protocol/` now records exactly what that work is, before anyone
starts.

**salman is still, after this, a tool with one protocol.** That is the honest cost, and it should
not be dressed up. Somebody whose need is a second fieldbus is not served by this decision and
will not be until v0.4. The roadmap says so, this ADR says so, and neither will pretend
otherwise in the meantime.

**Three documents gain a subcommand to correct.** `docs/CONFORMANCE.md`, `docs/ROADMAP.md` and
`CLAUDE.md` each say salman has seven subcommands. An eighth makes all three wrong on the day it
lands.

## Alternatives considered

**Building OPC UA or CANopen next.** It is the roadmap's v0.4 and the obvious answer to "salman
speaks one protocol". It lost on measurement: 7 242 lines and 133 tests bought Modbus, the
workspace has no protocol seam to add a second one to, the project file's YAML schema hard-codes
Modbus's four table names, and `ADR-0014` limits the whole exercise to read-only access. Set that
against a viewport that is mostly re-exposure of tested API, and the ratio is not close. It also
lost on audience: a second protocol serves a user who does not exist yet, while the viewport
serves the user who cloned the repository this morning.

**Building the desktop application now.** It is the milestone the README points at when it says
there is no screenshot. Rejected because a GUI is where a headless core goes to become untestable
if the core is not built first, and because the honest scope of a Tauri application — ladder and
FBD canvases, semantic diff, signed installers — is a milestone, not a next step. The monitor is
the part of it that can be built, tested and shipped in isolation, and the part the desktop
application would otherwise have to invent behind glass.

**Implementing the standard function library instead.** `docs/CONFORMANCE.md` calls it *"the
largest single gap in the language surface"*, and it is: not one standard *function* is
implemented, and the type checker already emits a diagnostic naming the `*_TO_*` conversion IEC
would use and then admitting salman does not have it. It is a real candidate and it should be
next after this. It lost on shape rather than on merit — it is hundreds of small independent
pieces of work, each needing a test and a `docs/CONFORMANCE.md` row, so it can be done
incrementally by anyone at any time, while a viewport is one coherent design that gets harder the
longer the CLI grows around it. Part of it is also blocked: `clippy.toml` bans every
transcendental, so `SIN` and `LOG` wait on the pinned `libm` crate — which `ADR-0005` names as
the intended substitute, and which is a new dependency and therefore a `LEGAL.md` §8 row, plus
a `deny.toml` licence-allowlist entry if its licence is not already on the list.

**Exposing PLCopen import on the command line.** The cheapest deliverable in the tree —
`Project::to_structured_text` already exists in `salman-plcopen`, is tested, has had its
document-controlled-identifier injection hazard found and fixed, and is called from nothing but
tests. One subcommand wrapping one tested function. Rejected as *the* answer because it is a
half-day of work rather than a direction, and because it converts a vendor export into text a
person then still cannot watch run. It should be done anyway, and it does not need an ADR.

**A language server instead.** `salman_vm::project`'s module doc already anticipates it — *"Every
front end — the command line, the test harness, and later the language server — wants the same
pipeline"* — and every diagnostic already carries a span, plus an IEC clause citation and the
dialect rule it applied where it has one (`Diagnostic::clause` and `Diagnostic::dialect_rule`
are both `Option`). That is most of what an LSP publishes. It lost because it needs an LSP transport
dependency and incremental reparse, because it serves editing rather than running, and because
the roadmap pairs it with the workbench at v0.3 where it can inherit this work rather than
duplicate it.

**Doing nothing until the cross-platform trace comparison is finished.** `docs/ROADMAP.md` lists
that as still owed by v0.1, and finishing what is owed before starting what is new is a defensible
rule. Rejected because the two are not in tension: the trace comparison is a workflow change of
perhaps fifty lines, and it should be done, but it is not a substitute for a milestone and holding
the project still for it would be scheduling by guilt.

## How this is enforced

**Nothing enforces this today, because nothing is built.** That is the honest state, and it is the
same state `ADR-0004` records for the network scope.

What will enforce it when the work starts:

- A `Capability` entry in `crates/salman-core/src/capability.rs` naming tests that exist.
  `tested_capabilities_must_cite_evidence` and `every_cited_test_exists_in_the_source_tree` refuse
  an entry whose tests were renamed or never written, and `docs/STATUS.md` is generated from the
  registry, so the claim and the evidence cannot drift.
- Tests for the parts of the decision that are promises rather than features: that a forced slot
  renders both values, that the force count is present whenever a force is, and that an ambiguous
  watch name is refused with the candidates named rather than resolved silently.
- `ADR-0014` continues to hold the live-device boundary in code, through
  `LinkError::WouldDriveALiveDevice`, and this decision adds nothing that could reach a device.
- Scope discipline is enforced by `LEGAL.md` §8, which requires a row for every dependency, and
  by `deny.toml`'s licence allowlist where a new crate's licence is not already on it. Neither
  is a strong gate on its own; together they make "no new dependency" a decision somebody has
  to visibly reverse rather than one that erodes.

If this ordering is ever revisited — if a user with a real OPC UA need arrives before v0.4 —
that is a new ADR superseding this one, not a change of plan in a commit message. The reason to
write this down was that the ordering looked arbitrary and turned out not to be.
