# One file, all the way through

This page follows `examples/conveyor/conveyor.st` from a text file to a passing test, and says
what each of salman's crates did to it on the way. It assumes you have written software and
have never written Structured Text, which is the common case and the one nothing else in this
repository is written for.

Every output on this page is real. Where it is truncated, it says so.

```bash
git clone https://github.com/celikgo/salman.git
cd salman
cargo build --release
```

Three commands, and the rest of the page explains them:

```bash
./target/release/salman check examples/conveyor/conveyor.st
./target/release/salman run   examples/conveyor/conveyor.st --until T#20ms --record Motor,State
./target/release/salman test  examples/conveyor/conveyor.st examples/conveyor/
```

---

## Part 1 — What the program says

### The scan cycle, which is the thing to understand first

A PLC does not run a program the way a process runs a program. It runs it **again and again,
for ever**, and one pass is a *scan*. Each scan:

1. **reads every input at once**, into a snapshot called the *process image*;
2. **runs the whole program** against that snapshot, so an input cannot change halfway through;
3. **writes every output at once**, from the image.

Everything else follows from that. There is no `main`, no event loop and no blocking. A
variable keeps its value from one scan to the next — that is how a program remembers anything
— and "wait two seconds" is not a `sleep`, it is a block that you call every scan and that
tells you when two seconds have gone by.

salman's clock for all this is **virtual**. The eight tests in this example cost about
35 000 scans and 35 seconds of *plant* time, and 0.05–0.08 s of real time on an Apple silicon
laptop — and they produce the same answer on every machine.

### The program

`examples/conveyor/conveyor.st` is a conveyor with a start-stop station, a motor starter with
a run-on delay, a part counter and a jam detector. It is 109 lines and holds three POUs — a
*program organisation unit* is IEC's word for a named unit of code: a program, a function
block or a function.

**A `FUNCTION_BLOCK` is a class with exactly one method, and the method is "run one scan".**
It has state that survives between calls, declared inputs and outputs, and you create
instances of it.

```iecst
FUNCTION_BLOCK Motor_Starter
VAR_INPUT
    Start   : BOOL;
    Stop    : BOOL;
    Run_On  : TIME;
END_VAR
VAR_OUTPUT
    Running : BOOL;
END_VAR
VAR
    Latch   : RS;
    Run_Off : TOF;
END_VAR
    (* RS is reset dominant: with both buttons pressed, the belt stops. *)
    Latch(S := Start, R1 := Stop);
    Run_Off(IN := Latch.Q1, PT := Run_On);
    Running := Run_Off.Q;
END_FUNCTION_BLOCK
```

`RS` and `TOF` are two of the ten **standard function blocks** IEC 61131-3 defines, and salman
implements all ten:

- **`RS` is a latch.** `S` sets it, `R1` resets it, `Q1` is its state. *Reset dominant*: if
  both are true, reset wins. That is a safety-shaped default and it is why the stop button
  beats the start button.
- **`TOF` is an off delay.** `Q` goes true the instant `IN` does, and stays true for `PT`
  after `IN` goes false. Here that is the run-on: the belt keeps moving for two seconds after
  stop, so it clears before it halts.

So `Motor_Starter` is four lines that mean *"latch the buttons, then hold the motor on for
`Run_On` after the latch drops"*.

**A `FUNCTION` is a pure function**, with no state between calls. Its return value is assigned
to its own name, which is IEC's convention:

```iecst
FUNCTION Percent_Of : INT
VAR_INPUT
    Count : INT;
    Batch : INT;
END_VAR
    IF Batch <= 0 THEN
        Percent_Of := 0;
    ELSE
        Percent_Of := Count * 100 / Batch;
    END_IF;
END_FUNCTION
```

**A `PROGRAM` is the top level.** `Conveyor` declares its field inputs and outputs, three
standard-function-block instances and one of its own, and then does the work:

```iecst
    (* The belt runs while the starter says so. *)
    Starter(Start := Start_PB, Stop := Stop_PB, Run_On := Run_On_Time);
    Motor := Starter.Running;

    (* One count per part, on the leading edge of the sensor. *)
    Part_Edge(CLK := Part_Sensor);
    Parts(CU := Part_Edge.Q, R := Stop_PB, PV := Batch_Size);
    Batch_Done := Parts.Q;
    Progress := Percent_Of(Count := Parts.CV, Batch := Batch_Size);

    (* A jam is the belt running with no part seen for the timeout. Each part
       restarts the timer, because the edge output is low again the next scan. *)
    Jam_Timer(IN := Motor AND NOT Part_Edge.Q, PT := Jam_Timeout);
    Jam_Lamp := Jam_Timer.Q;
```

Three more standard blocks, and one idea worth getting right because the obvious explanation
of it is wrong:

- **`R_TRIG` is a rising-edge detector.** `Q` is true for exactly *one scan*: the scan in which
  `CLK` first became true. However long the beam stays broken, `Part_Edge.Q` is true once.
- **`CTU` counts up.** `CU` is the count input, `R` resets, `PV` is the preset, `CV` is the
  current value, `Q` is true once `CV` reaches `PV`. It **edge-detects `CU` itself** — salman's
  implementation opens with `rising(fb, "CU", "CU_M")`, which is what IEC specifies — so a
  sensor held high for six scans counts one part, and feeding it `Part_Sensor` directly would
  work. You can check that: swap `Part_Edge.Q` for `Part_Sensor` on that line and all eight
  tests still pass.
- **`TON` is an on delay** — the mirror of `TOF`. `Q` goes true `PT` after `IN` went true, and
  false the instant `IN` does.

**So why is `Part_Edge` there at all?** For the jam timer, which is the one genuinely subtle
line in the file:

```iecst
    Jam_Timer(IN := Motor AND NOT Part_Edge.Q, PT := Jam_Timeout);
```

The edge output drops the timer's input for **exactly one scan** per part, however long the
beam is broken. Write `NOT Part_Sensor` there instead and a part that *stops* in the beam holds
`IN` false for ever: the belt is jammed and the jam lamp never lights, which is precisely the
case the detector exists to catch. That is not a matter of opinion, and it takes four lines to
show:

```yaml
- test: "a part stuck in the beam still raises a jam"
  pou: Conveyor
  steps:
    - { set: { Start_PB: true }, scans: 1 }
    - { set: { Part_Sensor: true }, advance: "T#11s", expect: { Jam_Lamp: true } }
```

That passes against `conveyor.st` as written, and against the `NOT Part_Sensor` version it
fails with `step 2: Jam_Lamp is FALSE at T#11s, expected TRUE`. Eleven seconds of plant time,
no conveyor, and a design question settled in about a second.

Then a `CASE` state machine, which exists to show `CASE`. `IF`/`ELSIF`, `CASE` and assignment
are the three statement forms a real program is mostly made of.

The header comment says what has to be said about anything in this domain:

> salman is not a safety tool. Nothing here is a safety function, and a real conveyor needs a
> hard-wired emergency stop that no program can override.

---

## Part 2 — What salman does to it

Eight stages, in two crates — three in `salman-lang`, five in `salman-vm`. **There is no flag
that dumps an intermediate representation** —
no `--emit tokens`, no `--emit ast`, no bytecode disassembler. That is worth saying plainly:
the sections below show each stage through the diagnostics it produces and through the types
in the source, not through a command you can run. If you want to *see* a stage, the unit tests
inside each module are where it is visible.

The one call that runs all of it is `salman_vm::project::build_all(files, dialect)` —
`build(name, text, dialect)` for a single file. `check`, `run` and `test` all go through it,
which is why they cannot disagree about what a program means.

### Stage 1 — lex · `salman-lang/src/lexer.rs`

```rust
lex(file: FileId, source: &str, dialect: &Dialect) -> (TokenStream, Diagnostics)
```

Characters to tokens. Nothing here knows what a name refers to; it knows what a name *is*.
The lexer owns literal forms, `$` escapes, based literals like `16#FF`, duration literals like
`T#2s`, `%` direct addresses, comment and pragma nesting, and identifier length. It is
`fuzz`ed daily and may never panic on malformed input.

Its diagnostics are `E01xx`. Break a based literal and you get one:

```
error[E0106]: `2` is not a base-2 digit
 --> lexerr.st:3:23
  |
3 |     Mask : BYTE := 2#12;
  |                       ^ digit out of range for this radix
  |

1 error
```

Note the caret is on the offending digit, not on the line. Every token carries a `Span`, and
every span survives to the end of the pipeline; that is what `salman-core` is for.

### Stage 2 — parse · `salman-lang/src/parser.rs`

```rust
parse(file: FileId, source: &str, stream: &TokenStream, dialect: &Dialect)
    -> (CompilationUnit, Diagnostics)
```

Tokens to a tree. Recursive descent, with error recovery — it reports several problems per
run rather than stopping at the first — and bounded nesting, because the input may be hostile.

Diagnostics are `E02xx`. Drop a semicolon:

```
error[E0202]: expected `;`, found `END_PROGRAM`
 --> syntaxerr.st:6:1
  |
6 | END_PROGRAM
  | ^^^^^^^^^^^ expected `;` here
  |

1 error
```

Every node gets a `NodeId`, and every side table downstream is indexed by it. When several
files are built as one program, `parse_from(..., first_id)` gives each file a disjoint id
range, so the tables stay disjoint by construction.

### Stage 3 — check · `salman-lang/src/sema.rs`, with `types.rs`

```rust
check(unit: &CompilationUnit, dialect: &Dialect) -> (Checked, Diagnostics)
```

The tree to *meaning*: name resolution, type checking, constant folding, and the static
recursion check that the memory layout depends on. This is where `Starter.Running` becomes a
reference to a field of a particular instance, `Batch_Size` becomes the constant `10`, and
`Percent_Of(Count := ..., Batch := ...)` becomes a call with bound parameters.

`types.rs` holds the rules **as data** — tables of permitted implicit conversions and operator
domains, not a cascade of `if`s. Its module doc explains why: *a rule you can print is a rule
an engineer can check against the standard.*

Diagnostics are `E03xx` (declarations and names) and `E04xx` (types), and this is where a
citation reaches the user:

```
error[E0401]: this target is INT, and this value is STRING[8]
 --> typeerr.st:6:14
  |
6 |     Count := Name;
  |              ^^^^ STRING[8] does not convert to INT on its own
  |
  = standard: IEC 61131-3:2013 Figure 12 "Supported implicit type conversions" (Ed 3.0)
  = requirement: The graph of conversions a conforming implementation performs without being asked, which is the set salman's type checker must not widen

1 error
```

The `= standard:` line is a locator into a document salman does not reproduce, and the
`= requirement:` line is salman's own paraphrase of what it is checking. Both come from an
entry in `crates/salman-core/src/clause.rs`, and a test in that file refuses to let the entry
exist unless it names a test that exists.

**`salman check` stops here.** On the conveyor:

```
$ ./target/release/salman check examples/conveyor/conveyor.st
examples/conveyor/conveyor.st: no errors
```

Exit code `0`. On a file with errors, exit code `1`.

### Stage 4 — compile · `salman-vm/src/compile.rs`

```rust
compile(unit: &CompilationUnit, checked: &Checked, dialect: &Dialect)
    -> (Option<Compiled>, Diagnostics)
```

The tree plus `Checked` become bytecode. Not an AST interpreter, because walking a tree per
scan makes the scan cost depend on source shape in ways that are hard to budget; not a
transpiler, because compiling to another language would put that language's arithmetic and
floating-point behaviour between salman and the determinism promise. `docs/adr/ADR-0006-bytecode-vm.md`
is the decision.

Twenty-eight opcodes, and the list is worth reading because it is short enough to hold in your
head and it tells you what the machine can do:

```
Const  Pop  Dup
LoadSlot  StoreSlot  LoadLocal  StoreLocal  LoadAddress  StoreAddress
LoadIndexed  LoadIndexedLocal  StoreIndexedLocal  StoreIndexed
BoundsCheck  CheckRange  CheckEnum  TruncateString
Binary  Unary  Convert
Jump  JumpIfFalse  JumpIfTrue
Call  CallLocal  CallNative  CallNativeLocal  Return
```

Four of those are the promises salman makes about declared types: `CheckRange` for a subrange
like `INT (0..100)`, `CheckEnum` for an enumeration, `TruncateString` for a `STRING[n]`, and
`BoundsCheck` for an array subscript. The first three are emitted by one function,
`Body::coerce`, whose own doc comment calls it *the single place a value becomes a value of a
declared type*; `BoundsCheck` is emitted where a subscript is compiled, because it is a check
on the index rather than on the value being stored. `docs/CONFORMANCE.md` enumerates every
site that stores into a declared destination, because a promise kept at some assignment sites
and not others is worse than one kept nowhere.

`CallNative` is how a native block is reached — the ten IEC standard function blocks, plus
`SEMA`, which ships for vendor compatibility and which salman never calls standard. Their
behaviour is in `salman-vm/src/stdfb.rs`; their shapes are in `salman-lang/src/stdlib.rs`, so
one definition serves both and they cannot disagree.

The compiler has its own diagnostic codes — `E0501` layout, `E0502` nothing to run, `E0503`
bad `AT %` location, `E0504` write to an input — declared in `compile.rs` rather than in
`salman-lang/src/codes.rs`, which is a trap worth knowing about if you are adding one.

### Stage 5 — lay out · `salman-vm/src/memory.rs`

Every instance gets a **static** address. `Starter` holds an `RS` and a `TOF`, each of which
holds its own state, and all of it is laid out at compile time — no instance is allocated
during a scan. This is why the static recursion check in stage 3 is not a nicety: a recursive
POU has no finite layout.

The process image (`%I`, `%Q`, `%M`) lives here too, at a fixed 4096 bytes per area. A variable
declared `AT %QX0.0` **is** that location — it gets no slot of its own, because a slot would be
a copy, and a copy of an input is correct right up until the moment it matters.

### Stage 6 — schedule and scan · `salman-vm/src/task.rs`

`Runtime`, `TaskConfig`, `TaskTrigger`, `StepOutcome`. Cyclic, event and freewheeling tasks,
with priorities and overrun detection.

`conveyor.st` declares no `CONFIGURATION`, which on a real controller could not happen — a
configuration is what binds a program to a task. salman gives every `PROGRAM` in such a file
one freewheeling task of its own, in declaration order. That is *salman policy* number 18 in
`docs/CONFORMANCE.md`, a convenience rather than a standard rule, and it is what makes
`salman check` and `salman run` useful on a single file. It also explains the first two lines
of a run:

```
posture: OBSERVE — `salman run` executes on the simulation runtime and writes to no device
task Conveyor_freewheeling priority 0 (Freewheeling)
```

A freewheeling task runs again as soon as it finishes, so on a virtual clock it needs a
modelled duration — zero would mean time never advances and no timer would ever fire. salman
uses one millisecond (`FREEWHEEL_DEFAULT_SCAN`), which is policy 19 and is explicitly **not a
measurement and not a claim about any controller**. It is why `--until T#20ms` gives 21 scans.

### Stage 7 — execute · `salman-vm/src/exec.rs`

```rust
execute(program, memory, clock, routine, base, limits) -> Result<Executed, Fault>
```

It **faults, it does not panic**. A subrange violation, a division by zero, an array index out
of bounds — each produces a `Fault` rather than a crash, and each says as much as it can: a
subrange violation names the variable, the value and the declared bounds; an out-of-range
subscript names the index and the bounds. Each scan has an instruction budget, and that budget
*is* salman's watchdog — `FaultKind::InstructionBudgetExceeded` says so in its own doc
comment.

```
$ ./target/release/salman run examples/conveyor/conveyor.st --until T#20ms --record Motor,State
posture: OBSERVE — `salman run` executes on the simulation runtime and writes to no device
task Conveyor_freewheeling priority 0 (Freewheeling)
21 scans, simulation time T#20ms
  Conveyor_freewheeling: 21 scans, 0 overruns, 74..74 instructions
```

`74..74 instructions` is the minimum and maximum instruction count across all scans. They are
equal here because the conveyor takes the same path every scan, which is what you want from
control logic and what a scan-time budget depends on.

### Stage 8 — the trace · `salman-vm/src/trace.rs`

The same run continues:

```
# salman trace format 1
# salman version: 0.1.0
# seed: 0
# clock: virtual (reproducible)
# fingerprint: 33c84b2277b952e766e628dace3e1bbf3f8a2091b6b2354f0a0d0830b85bbd88
# samples: 21
scan	time	task	Motor	State
1	T#0s	0	FALSE	0
2	T#1ms	0	FALSE	0
3	T#2ms	0	FALSE	0
```

Truncated after three rows; there are 21.

Six header lines, a tab-separated column header, then one row per sample: scan number,
simulation time as an IEC duration literal, task index, and one column per recorded signal.
`Motor` is `FALSE` throughout because nothing pressed the start button — `salman run` drives no
inputs. Making the inputs move is what the test file is for.

Everything in that header is fixed, declared, or derived from the run. **Nothing is ambient**:
no hostname, no wall-clock time, no path, no username. The fingerprint is SHA-256 over a
canonical *binary* encoding of the values, not over the rendered text, so `Value::Int(1)` and
`Value::Dint(1)` cannot hash the same. Run it twice and the bytes are identical:

```
$ salman run … --trace /tmp/a.trace && salman run … --trace /tmp/b.trace && cmp /tmp/a.trace /tmp/b.trace
```

One caveat on `--record`, because it will catch you: it accepts a `%` address, an exact full
slot name (`--record Conveyor.Parts.CV` works), or a bare *final* segment (`--record Motor`).
A partial dotted path is refused — `--record Parts.CV` gives *"no variable called Parts.CV"*.
The `record:` list in a test file uses a different resolver that matches any dotted suffix, so
`Parts.CV` works there, which is why the golden test below can name it.

---

## Part 3 — Testing it

`salman test` is where the whole thing earns its keep, because the interesting questions about
a conveyor are questions about *time*, and time is what a hardware test rig makes expensive.

### The test file

`examples/conveyor/conveyor.salman-test.yaml`. A test file is one test or a list of them, and
`deny_unknown_fields` is on, so a typo is an error rather than a silently ignored key.

| Key | Type | Meaning |
|---|---|---|
| `test` | string, **required** | The name, shown in the report and in the JUnit XML |
| `pou` | string | Which POU names resolve against, when a name is ambiguous |
| `given` | map | Variables written before the first step |
| `steps` | list | The steps, in order |
| `record` | list of strings | Signals to record — for a golden-trace test |
| `golden` | string | The golden trace file, relative to the test file |
| `seed` | integer | Recorded in the trace. Defaults to 0 |
| `skip` | string | A reason. Present means skipped — *"a skipped test with no reason is a test nobody will fix"* |

And a step:

| Key | Type | Meaning |
|---|---|---|
| `set` | map | Variables to write before running |
| `force` | map | Variables to force, which the program then cannot overwrite |
| `release` | list | Forces to release |
| `scans` | integer | Scans to run |
| `advance` | string | Simulation time to advance, as `"T#5s"` |
| `expect` | map | Variables to check after running |
| `note` | string | Shown when this step fails |

Values are written as IEC literals and mean exactly what they mean in source: `T#5s` is a
duration, `16#FF` is a byte, `TRUE` is a `BOOL`.

Here is the test that pays for the whole design:

```yaml
- test: "the belt runs on for two seconds after the stop button, then stops"
  pou: Conveyor
  steps:
    - { set: { Start_PB: true }, scans: 1, expect: { Motor: true } }
    - set: { Start_PB: false, Stop_PB: true }
      scans: 1
      expect: { Motor: true }
      note: "the run-on TOF holds the motor while the belt clears"
    - { advance: "T#1s999ms", expect: { Motor: true } }
    - { advance: "T#2ms", expect: { Motor: false } }
```

It asserts the motor is still on at 1.999 s and off two milliseconds later — the boundary
checked from both sides. On real hardware that is a person with a stopwatch. Here it is 2003
scans of plant time and no wall time.

The last test in the file is a **golden-trace test**: it names `record:` and `golden:`, runs
its steps, and compares the resulting trace against the committed
`examples/conveyor/conveyor.trace` byte for byte. That file is text you can read, and its diff
in a pull request tells you what changed about the machine's behaviour. Regenerate it with
`--update-golden`, and then read the diff — regenerating without reading converts the strongest
test in the repository into a rubber stamp.

### Running it

```
$ ./target/release/salman test examples/conveyor/conveyor.st examples/conveyor/
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

Roughly 35 000 scans and 35 seconds of simulated plant time, in 0.05–0.08 s of real time on
an Apple silicon laptop (three runs). Exit code `0`. `--junit report.xml` writes a JUnit XML report, so the
same command is a CI job.

A failure names the step and the value:

```
 FAIL  a deliberately wrong expectation, to show what a failure looks like  (1 scans, T#0s)
         step 1: Motor is TRUE at T#0s, expected FALSE

1 tests: 0 passed, 1 failed, 0 errored, 0 skipped
```

Exit code `1`. A `note:` on the step is printed alongside, which is what notes are for.

---

## Which crate did what

For `salman test examples/conveyor/conveyor.st examples/conveyor/`, exactly five of the
thirteen crates ran:

| Crate | What it did |
|---|---|
| `salman-cli` | parsed the command line, found the `.salman-test.yaml` files, printed the report, chose the exit code |
| `salman-lang` | lexed, parsed and type-checked the source |
| `salman-vm` | compiled it to bytecode, laid out memory, ran the scans, recorded the trace |
| `salman-test` | read the YAML, drove the steps, checked the expectations, compared the golden trace, rendered the report |
| `salman-core` | spans, diagnostics, identifiers, values, durations, and the SHA-256 behind the fingerprint |

The other eight did nothing, and it is worth knowing why, because it is the shape of the
project: `salman-modbus` and `salman-modbus-net` are the protocol; `salman-capture`,
`salman-findings` and `salman-analyse` are the packet-capture path; `salman-project` and
`salman-link` bind a real device's registers to the process image; `salman-plcopen` reads and
writes PLCopen XML and is not reachable from the CLI at all.

Nothing in the five rows above touched a network, a wall clock, or a licence server. The
clock they did touch is the virtual one, which is the point.

---

## Where to go next

- **Break something.** Change `T#2s` to `T#3s` in `conveyor.st` and run the tests. One fails —
  *"the belt runs on for two seconds after the stop button, then stops"* — with
  `step 4: Motor is TRUE at T#2s2ms, expected FALSE`. The message names the boundary that
  moved, which is the whole argument for testing a machine this way.
- **Read `docs/CONFORMANCE.md`.** Especially `## salman policy` — thirty places where
  IEC 61131-3 did not settle a question and salman had to, each with what the question was,
  what salman does, and why it is a policy rather than a requirement.
- **Read `examples/capture/`** for the other half of the tool: reading a packet capture and
  saying what happened on it.
- **`.claude/skills/`** holds the working knowledge for changing any of this — the language
  pipeline, the determinism rules, the protocol seam, the citation policy, the release path.
- **`CONTRIBUTING.md`** for the test tiers, and for the question the review asks of any claim:
  *which test backs it?*

**salman is not a safety tool.** The runtime described on this page is for development,
testing and simulation. It is not certified under IEC 61508, IEC 62061, ISO 13849 or anything
else, and it is not for controlling machinery. See [`../LEGAL.md`](../LEGAL.md).
