# ADR-0006: A bytecode VM, not an AST interpreter and not a transpiler

- **Status**: Accepted
- **Date**: 2026-08-18
- **Deciders**: the salman authors

## Context

salman promises that the same project, with the same inputs and the same seed, produces
a byte-identical trace on Linux, macOS and Windows. Everything about how Structured Text
is executed is downstream of that promise.

Three further forces bear on the choice of execution engine.

A scan has to be **budgetable**. A controller has a watchdog; a simulator that hangs a
test run instead of stopping a runaway loop is worse than useless, because the failure
arrives as a timeout rather than as a diagnostic naming the program.

The engine has to **never panic**. It runs code compiled from files salman did not write.
A panic would also embed a source line number in its message, which changes when the file
is edited and would break the determinism gate on a commit that touched nothing else.

salman has to be able to **state what every operation does**. Integer overflow wraps.
Integer division by zero is a fault, not a zero. Real division by zero follows IEEE 754.
None of those three is fixed by IEC 61131-3:2013 (Edition 3.0) in a way salman could
verify from a public source, so salman decides them, writes them down, and tests them.
That is only possible if no other language's semantics sit in the middle.

Edition 3.0 was withdrawn on 2025-05-22 and superseded by IEC 61131-3:2025 (Edition 4.0).
salman targets Edition 3.0 because it is the edition its public sources let it verify.

## Decision

salman compiles Structured Text to a bytecode and interprets it. The instruction set
lives in `crates/salman-vm/src/bytecode.rs` and the interpreter in
`crates/salman-vm/src/exec.rs`. The loop is single-threaded, reads no clock, and iterates
no hash map.

Three design consequences follow directly, and are part of the decision rather than
details of it.

**Absolute static slot addressing, with no stack frames.** Every slot reference in the
bytecode is an absolute index — `Op::LoadSlot(u32)`, `Op::StoreSlot(u32)`. Every function
and every function block instance has one permanent home in memory, which is how a real
controller works and is what makes a scan's memory cost knowable before it runs. This is
sound only because IEC 61131-3:2013 §6.6 "Program organization units (POUs)" (Ed 3.0)
does not permit a POU to invoke itself, directly or through a cycle, and because salman
rejects recursion statically rather than relying on it not happening.

**Operand types are decided at compile time and carried in the instruction.**
`Op::Binary { op, ty }` and `Op::Unary { op, ty }` name the elementary type they operate
on. The interpreter never infers an operation from the values it finds on the stack. The
arithmetic of a program therefore depends on the declaration the engineer wrote, not on
the data that happened to arrive; a value of an unexpected type is a
`FaultKind::TypeMismatch`, never a silent coercion.

**A per-scan instruction budget.** `ExecLimits::max_instructions` defaults to ten million
and produces `FaultKind::InstructionBudgetExceeded`. `WHILE TRUE DO ; END_WHILE` stops the
task and says why. The operand stack and call depth are bounded the same way, and each
routine's peak stack is computed at compile time and recorded in `Routine::max_stack`.

## Consequences

An interpreter is slower than compiled code. salman has published **no performance
numbers** and will not claim any until they are measured. `.github/workflows/perf.yml`
today measures cold start, peak resident set, binary size and test-suite wall time against
`perf-budget.toml`; none of those is interpreter throughput, and there is no VM benchmark
in this repository at all.

The instruction set is now a compatibility surface. `Op`, `Routine` and `Program` are
public in `salman-vm`. Nothing serialises bytecode to a file yet, so the surface is cheap
to change today — the day a compiled artefact, a debugger or a third crate reads it, it
stops being cheap.

If an ahead-of-time backend is ever added, salman maintains two implementations that must
agree, and agreement has to be demonstrated by a differential test suite that does not
exist. That cost is real and is a reason to defer the backend, not a reason to pretend it
would be free.

Some failures that a compiler would catch arrive at runtime instead. `BinOp::Pow` on
integers is `FaultKind::Unsupported` in the interpreter because the front end is expected
to convert to a real first; if it ever fails to, the program faults mid-scan rather than
failing to compile.

The recursion rejection is, at 0.0.1, a design commitment and not yet a check. There is no
compiler from the AST to the bytecode in this repository, so nothing rejects recursion
today. The static addressing scheme is only sound once that check exists.

## Alternatives considered

**A tree-walking interpreter.** The simplest thing that works: no compile step, no second
representation, and errors keep their source position for free. It lost because per-scan
cost then depends on the shape of the source in ways that are hard to budget, and because
an instruction count — the thing the watchdog is expressed in — has no natural meaning
when the unit of work is a tree node reached through a pointer chase.

**Transpiling to C.** Fast, and the resulting artefact runs anywhere a C compiler does.
It lost on determinism: C's integer promotions, its undefined behaviour on signed
overflow, and the floating-point flags of whichever compiler the user happens to have all
sit between salman and the trace it promises. It also puts a C toolchain in the
dependency list of an engineering tool that otherwise needs only Rust.

**Ahead-of-time compilation through LLVM.** The RuSTy project takes exactly this route —
"a structured text compiler written in Rust, utilizing the LLVM framework for native code
compilation" (<https://github.com/PLC-lang/rusty>). The trade-off is real and is not a
criticism of them: they get native speed and a mature optimiser, and salman gets to decide
and state what every operation does. Two projects can reasonably want different halves of
that. salman also avoids a large native build dependency, which matters for a tool meant
to be installed on an engineering laptop.

**WebAssembly as the target.** Attractive — portable, sandboxed, and with a fuel mechanism
that resembles the instruction budget. It lost because the budget, the trap behaviour and
the floating-point rules would then be the host runtime's rather than salman's, and
because embedding a Wasm runtime is a substantial supply-chain surface for a project whose
first engineering rule is that untrusted input is treated as hostile.

## How this is enforced

* `crates/salman-vm/src/task.rs`, `the_scan_watchdog_stops_a_program_that_never_ends` —
  the instruction budget stops a non-terminating program.
* `crates/salman-vm/src/task.rs`, `statistics_record_the_instruction_cost_of_a_scan` — a
  scan's cost is measured in instructions, which is what makes the budget meaningful.
* `crates/salman-vm/src/task.rs`,
  `the_same_configuration_run_twice_produces_the_same_trace_fingerprint`.
* `crates/salman-vm/src/bytecode.rs`,
  `routines_and_slots_are_found_by_name_case_insensitively` — the static layout is
  addressed by index and named for diagnostics only.
* `crates/salman-vm/src/clock.rs`, `two_clocks_advanced_the_same_way_agree_exactly`.

Three things are **not** enforced, and saying otherwise would be the failure this project
exists to avoid. `crates/salman-vm/src/exec.rs` has no test module at all: its arithmetic
policies — wrapping integer overflow, division by zero as a fault, IEEE 754 for reals —
are stated in module documentation and exercised only indirectly through the task and
standard-function-block tests. Nothing rejects recursion, because nothing compiles to
bytecode yet. And `.github/workflows/determinism.yml` does not yet compare a trace across
platforms; it says so on every run, in a step named for the gap.
