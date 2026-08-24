---
name: extending-structured-text
description: How a Structured Text language feature moves through salman — lexer, parser, AST, sema, types, salman-vm compile, exec — which crate owns which decision, the stable diagnostic-code convention in codes.rs, and the three-step ritual that must be followed when IEC 61131-3 is ambiguous (mark it `salman policy` in the source, record it in docs/CONFORMANCE.md, give it a named test). Use when adding or changing a statement, expression, type, literal, declaration form, standard function block or diagnostic anywhere in salman-lang or salman-vm; when a construct parses but does not compile; or when you are about to decide something the standard does not settle.
---

# Adding a language feature

Structured Text is not a small language and IEC 61131-3 is not a clear one. This skill is
about doing both parts well: getting a feature through seven stages in the right order, and
handling the moment — which arrives more often than you would expect — when the standard
does not answer the question you need answered.

If you read nothing else, read **The ritual**, below. It is the thing that makes this project
trustworthy, and it is the thing a contributor will not infer.

## The pipeline

Two crates, seven stages. The four front-end stages each return diagnostics alongside their
output rather than aborting, so one bad declaration produces one message and the rest of the
file is still checked. Execution reports a `Fault` instead. Nothing panics.

| Stage | File | Entry point | In → out |
|---|---|---|---|
| lex | `salman-lang/src/lexer.rs` | `lex(file, source, dialect) -> (TokenStream, Diagnostics)` | `&str` → tokens |
| parse | `salman-lang/src/parser.rs` | `parse(file, source, stream, dialect) -> (CompilationUnit, Diagnostics)` | tokens → AST |
| check | `salman-lang/src/sema.rs` | `check(unit, dialect) -> (Checked, Diagnostics)` | AST → resolutions, types, folded constants |
| compile | `salman-vm/src/compile.rs` | `compile(unit, checked, dialect) -> (Option<Compiled>, Diagnostics)` | AST + `Checked` → bytecode + memory layout |
| lay out | `salman-vm/src/memory.rs` | `Memory`, `ProcessImage`, `SlotId` | static instance layout, no run-time allocation |
| schedule | `salman-vm/src/task.rs` | `Runtime`, `StepOutcome`, `TaskTrigger` | one scan at a time, with the process image |
| execute | `salman-vm/src/exec.rs` | `execute(program, memory, clock, routine, base, limits) -> Result<Executed, Fault>` | bytecode → effects, or a `Fault` |

The one-call front door for all of it is `salman_vm::project::build_all(files, dialect)` —
parse every file into one node-id space, join, check, compile — returning a `Build`.
`salman_vm::project::build(name, text, dialect)` is the single-file form. `salman check`,
`salman run` and `salman test` all go through these, which is why they cannot disagree about
what a program means.

### What each stage owns, and only that stage

- **`lexer.rs` owns what a token *is*.** Literal shapes, `$` escapes, based literals,
  duration and date literals, `%` direct addresses, comment and pragma nesting, identifier
  length. If your feature needs new surface syntax at the character level, it starts here
  and `token.rs` gains a `TokenKind`, `Keyword` or `Punct`.
- **`parser.rs` owns shape.** Recursive descent with error recovery and bounded nesting. It
  decides what sequence of tokens is a statement, an expression, a declaration. It does *not*
  know what a name refers to and must not try.
- **`ast.rs` owns the tree, and the node-id space.** Every side table downstream —
  resolutions, folded constants, inferred types — is indexed by `NodeId`, so a multi-file
  project hands each file a disjoint id range at parse time (`parse_from(..., first_id)`).
  Renumbering an AST afterwards would be wrong the first time a node was missed.
- **`sema.rs` owns meaning.** Name resolution, type checking, constant folding, the static
  recursion check the memory layout depends on. It produces `Checked`, and `Checked` is what
  the compiler reads instead of re-deriving anything.
- **`types.rs` owns *what is legal*, written as data.** Tables of permitted implicit
  conversions and operator domains, not a cascade of `if`s. Its module doc explains why: *a
  rule you can print is a rule an engineer can check against the standard.* If your feature
  adds a conversion or an operator domain, it is a table row here, not a special case in
  `sema.rs`.
- **`dialect.rs` owns what varies between vendors.** See below.
- **`compile.rs` owns *what it does*.** Bytecode selection, instance layout, the temporaries
  a construct reserves. `Body::coerce` is, by its own doc comment, *the single place a value
  becomes a value of a declared type* — every subrange check, string truncation and implicit
  conversion goes through it. If you add a new site that stores into a declared destination
  and do not route it through `coerce`, you have created an invisible hole. `docs/CONFORMANCE.md`
  enumerates every such site under *Every site that stores a value into a declared destination*
  precisely because a promise kept at some assignment sites and not others is worse than one
  kept nowhere.
- **`exec.rs` owns runtime behaviour and faults.** It faults rather than panics — always.
  Each scan has an instruction budget, and that budget *is* salman's watchdog:
  `FaultKind::InstructionBudgetExceeded` says so in its own doc comment. `ExecLimits` carries
  `max_instructions`, `max_stack` and `max_call_depth`.
- **`salman-core` owns the vocabulary**: `Span`, `SourceMap`, `FileId`, `IdentKey`,
  `Diagnostic`, `DiagCode`, `Value`, `ElementaryType`, `Duration`. Nothing about IEC syntax
  lives there.

**The seam most often crossed by mistake** is between `sema.rs` and `compile.rs`. "Is this
legal?" is `salman-lang`. "What does it do?" is `salman-vm`. When a change needs both — and
a real feature usually does — it needs a test in both.

## The ritual

**When IEC 61131-3 is ambiguous, salman does not guess silently.**

This is not a style rule. It is the reason a reader can trust a table in
`docs/CONFORMANCE.md`, and it is what separates salman from a compiler that happens to have
picked something. Thirty decisions are recorded this way today.

Three steps. All three, in the same pull request.

### 1. Mark it in the source, at the decision

A comment or doc comment containing the exact string `salman policy`, sitting on the code
that implements the choice — not in a header, not in a design document. There are 36 such
markers across `salman-lang`, `salman-vm` and the CLI's integration tests. The house form:

```rust
/// **salman policy.** No standard default could be verified from a public
/// source: one vendor documents `DINT`, another "the smallest possible type".
/// salman makes an untyped literal take the type its context requires and fall
/// back to `DINT` (`LREAL` for reals) when there is no context.
```

Say what the question is, what salman does, and *why it is a policy* — which always means
naming what you could not verify, or which two sources contradict each other. "Vendors
differ" without saying which vendors is not a reason, it is a shrug.

### 2. Record it in `docs/CONFORMANCE.md`, under `## salman policy`

Entries are numbered, and the shape is a convention rather than a template — most carry
**What salman does.** and **Why a policy.**, and about a third also open with **Question.**
Follow the full form; it is the clearest one and the one the earlier entries use:

```markdown
### 31. Whether a FOO may appear inside a BAR

**Question.** One sentence, phrased as the question a user would ask.

**What salman does.** One or two sentences. Concrete.

**Why a policy.** What could not be verified, or which sources disagree and how.
Name them. Then the test names, so a reader can go and look.
`sema.rs: a_foo_inside_a_bar_is_refused_by_a_salman_rule`.
```

The source location and the test names come at the end of the *Why a policy* paragraph rather
than under a heading of their own.

If the honest answer is that salman *believes* something it could not confirm, the entry does
not belong under `## salman policy` at all — it belongs in `## UNVERIFIED`, which is for
things salman has no choice to defend, only an unverified belief it is acting on. Each
`UNVERIFIED` entry carries a *"What would settle it"* line. That distinction is load-bearing:
policy 5 (unary versus exponentiation, where salman disagrees with CODESYS and Beckhoff)
rests entirely on one inference recorded in `UNVERIFIED`, and the policy entry says so.

`docs/CONFORMANCE.md` is written by hand and calls itself the weakest link in the chain. That
is the reason the third step exists.

### 3. Give it a named test

A test function whose name is a sentence describing the decision, cited from the
`CONFORMANCE.md` entry by file and function name. Real examples:

```
bool_widening_is_a_setting_because_the_sources_contradict_each_other
the_strict_dialect_rejects_lowercase_hexadecimal_digits
negative_durations_are_accepted_by_the_generic_dialect_and_refused_by_the_strict_one
duplicate_case_labels_are_refused_by_a_salman_rule
the_bounds_of_a_for_loop_are_evaluated_once_at_entry
a_call_with_enable_false_does_not_happen_at_all
a_structure_field_may_be_called_en_or_eno
a_subrange_bound_is_enforced_when_the_value_is_not_a_constant
```

The name is the specification. Someone reading a CI failure will see only that string.

**If the decision also justifies an IEC citation**, register it in
`crates/salman-core/src/clause.rs` — and then the test name is checked mechanically:
`every_cited_test_exists_in_the_source_tree` greps for `fn <name>(` in the file you named,
and `every_citation_names_at_least_one_test` refuses an entry with no test at all. See the
`citing-the-standard` skill; the citation policy is a legal constraint, not a stylistic one.

**If the decision is a capability rather than a rule**, the same applies through
`crates/salman-core/src/capability.rs`, which generates `docs/STATUS.md`.

A consequence worth internalising: **renaming a test can break a registry entry in a
different crate.** That is deliberate.

## Diagnostic codes

`crates/salman-lang/src/codes.rs` is the register, and its own header states the rule:

> Codes appear in users' CI filters and lint suppressions, so once published a code keeps its
> meaning for ever. Retiring one means never reusing the number.

The documented ranges, and how full each is today:

| Range | Meaning | In use | Next free |
|---|---|---|---|
| `E01xx` | lexical | `E0101`–`E0116` | `E0117` |
| `E02xx` | syntactic | `E0201`–`E0211` | `E0212` |
| `E03xx` | declarations and symbols | `E0301`–`E0325` | `E0326` |
| `E04xx` | types | `E0401`–`E0415` | `E0416` |
| `E05xx` | compilation and layout — **declared in `salman-vm/src/compile.rs`, not here** | `E0501`–`E0504` | `E0505` |
| `W0xxx` | warnings | `W0101`, `W0102`, `W0201`, `W0301`, `W0302` | per group |
| `U01xx`–`U03xx` | not implemented, in the band of the stage that refuses it | `U0101` lexer, `U0201` parser, `U0301` checker | per band |
| `U05xx` | not implemented, refused by the compiler — **also in `compile.rs`** | `U0501` | `U0502` |

Two things about that table that will catch you:

1. **`codes.rs` is not the only place codes are declared.** `crates/salman-vm/src/compile.rs`
   owns the whole `x05xx` band: `E_LAYOUT` (`E0501`), `E_NOTHING_TO_RUN` (`E0502`),
   `E_BAD_LOCATION` (`E0503`), `E_WRITE_TO_INPUT` (`E0504`) and `U_NOT_COMPILED` (`U0501`).
   The band exists because compilation is the stage after the four `salman-lang` numbers, and
   because `salman-lang` cannot see `salman-vm`.
2. **`diagnostic_codes_are_unique` does not span crates**, and cannot: it is a
   hand-maintained array inside `codes.rs`'s own test module. The check that does span them
   reads the source — `no_diagnostic_code_means_two_things_in_this_workspace` in
   `crates/salman-core/src/diag.rs` scans every `pub const … DiagCode("…")` under
   `crates/*/src/` and fails if one number carries two names, printing both.

   That test exists because the workspace shipped 0.1.0 with `U0301` meaning two things:
   `salman_lang::codes::U_REFERENCES`, which covers `REF_TO`, `^` and `?=`, and the
   compiler's not-implemented refusal. The compiler's moved to `U0501`. So: **grep the whole
   tree before you take a number** — and add your constant to the
   `diagnostic_codes_are_unique` array as well, because the fast local check is still worth
   having.

### Emitting one

`salman_core::diag` is a fluent builder. Attach everything you have:

```rust
Diagnostic::error(codes::E_TYPE_MISMATCH, "this target is INT, and this value is STRING[8]")
    .with_primary(span, "STRING[8] does not convert to INT on its own")
    .with_secondary(decl_span, "declared here")
    .with_note("a conversion function would be needed, and salman implements none yet")
    .with_clause(clause::FIGURE_IMPLICIT_CONVERSIONS)
    .with_dialect_rule("iec61131-3:2013-strict refuses this")
    .with_suggestion("write the conversion explicitly", edits)
```

`with_clause` is what puts the `= standard:` and `= requirement:` lines under the caret.
`with_dialect_rule` is mandatory in spirit for anything a dialect decides: **every diagnostic
must name the rule it applied**, so a user who moves code between tools can see which
position salman took. Use `Diagnostic::warning` for `W0xxx`.

What that produces, from a real run:

```
error[E0401]: this target is INT, and this value is STRING[8]
 --> typeerr.st:6:14
  |
6 |     Count := Name;
  |              ^^^^ STRING[8] does not convert to INT on its own
  |
  = standard: IEC 61131-3:2013 Figure 12 "Supported implicit type conversions" (Ed 3.0)
  = requirement: The graph of conversions a conforming implementation performs without being asked, which is the set salman's type checker must not widen
```

## Dialects

`crates/salman-lang/src/dialect.rs` holds two profiles: `Dialect::generic` (`DialectId::Generic`,
spelled `generic`) and `Dialect::strict_iec` (`DialectId::StrictIec`, spelled
`iec61131-3:2013-strict`). Vendor dialects — CODESYS, TwinCAT, Siemens SCL, OpenPLC, Beremiz —
are **not implemented**, and `DialectId` deliberately does not name them.

A dialect is a struct of settings, not a fork of the parser. `Dialect::strict_iec()` is
written as `{ id: StrictIec, ..Self::generic() }` with the differences spelled out, so the
diff between the two dialects is readable in one screen. When a feature is one that vendors
disagree about, the answer is usually a new field on `Dialect`, a `salman policy` entry
explaining the contradiction, and a pair of tests — one per dialect. Policies 2, 3 and 4 in
`docs/CONFORMANCE.md` are all of this shape.

The refusal code for a dialect-rejected construct is `E0115` (`E_DIALECT_REJECTS`) at the
lexical level; elsewhere the diagnostic carries `with_dialect_rule`.

## Where tests go

- **Unit tests are inline**, in `#[cfg(test)] mod tests` at the bottom of the file that owns
  the behaviour. `lexer.rs`, `parser.rs`, `sema.rs`, `types.rs`, `exec.rs` and `stdfb.rs` all
  carry large ones. **`compile.rs` carries none** — across 2 775 lines there is no
  `#[cfg(test)]` at all, and the compiler is tested end to end from `crates/salman-cli/tests/`
  instead. If you are adding to the compiler, that is where your test goes.
- **End-to-end tests live in `crates/salman-cli/tests/`**: `semantics.rs`, `constraints.rs`,
  `diagnostics.rs`, `located.rs`, `project.rs`, `conveyor_example.rs`. These drive the whole
  pipeline the way a user does, and `docs/CONFORMANCE.md` cites them constantly —
  `semantics.rs` is its second most-cited file, after `sema.rs`.
- **Golden artefacts** — traces, reports, the compatibility matrix — regenerate with
  `--update-golden` or `SALMAN_UPDATE_GOLDEN=1`, and the diff must be read before committing.

Every crate re-allows `unwrap`/`expect`/`panic` under `cfg(test)`; in library code they are
`deny`. A parser or decoder may never panic on malformed input, which is rule 7, and which
the fuzz targets in `fuzz/fuzz_targets/` exist to attack.

## A worked example: `EN` / `ENO`

This is the shape of a feature that touched every stage, and it is recorded in
`docs/CONFORMANCE.md` under *Where salman accepts something and does not mean it*. Before it,
salman accepted a `VAR_INPUT` called `EN` and gave it no meaning, so `F(EN := FALSE, N := 7)`
called `F` anyway — a construct accepted without carrying its meaning, which is the worst
category of defect this project recognises.

The order the work went in:

1. **Read the standard's shape.** Table 18 makes `EN`/`ENO` part of the *calling convention*,
   not something a POU declares. That single fact determines everything below; getting it
   wrong would have produced a plausible feature that was wrong in a way tests would not
   catch.
2. **`sema.rs`** — `EN` and `ENO` become available on every function and function-block call
   without being declared, and no POU may declare a variable or file-scope global with either
   name (`E0324`).
3. **The deliberate asymmetry, and its policy entry.** The reservation covers POU variables
   and globals and **not** structure fields, because a structure is never callable and
   refusing `Flags.EN` would invent a restriction IEC 61131-3 does not have. That is a
   `salman policy`, recorded, with `a_structure_field_may_be_called_en_or_eno` to prove the
   field still works.
4. **`compile.rs`** — a call with `EN` false does not happen, and does not bind its inputs
   either, because binding the inputs is part of the call. `ENO` is true when the call
   happened and true whenever `EN` is absent.
5. **A second policy where the standard ran out.** `EN` on a call *whose result is used* is
   refused (`U0501`): with `EN` false there is no call and therefore no result, and salman
   will not invent one.
6. **Named tests for each**: `a_call_with_enable_false_does_not_happen_at_all`,
   `a_call_that_does_not_happen_does_not_write_its_inputs_either`,
   `enable_out_reports_whether_the_call_happened`, `a_variable_may_not_be_called_en_or_eno`,
   `enable_on_a_call_whose_result_is_used_is_refused_rather_than_invented`.
7. **`docs/CONFORMANCE.md` rewritten**, including deleting the row that said `EN` was accepted
   without meaning.

Note step 3 and step 5: two ambiguities surfaced *during* the work, and both got the full
ritual. That is normal. A feature that surfaces no ambiguity in this language is a small one.

## Adding a standard function block

`salman_lang::stdlib::NativeBlock` is the enum of the ten IEC standard function blocks — `Sr`,
`Rs`, `RTrig`, `FTrig`, `Ctu`, `Ctd`, `Ctud`, `Tp`, `Ton`, `Tof` — plus `Sema`, which
**salman never calls standard**: it ships for vendor compatibility, its doc comment says so,
and `docs/CONFORMANCE.md` has an entire section making sure nobody reads it as one. Behaviour
lives in `crates/salman-vm/src/stdfb.rs`, which carries several `salman policy` markers of its
own, mostly about timers: what happens when `PT` changes mid-timing, what a negative `PT`
means, and the two `TP` ambiguities.

The standard *function* library — `ABS`, `SQRT`, `SEL`, `MAX`, `MIN`, `LIMIT`, `MUX`, the
`*_TO_*` conversions, the shifts, the string functions — is **not started**. Not one is
implemented. A narrowing-assignment diagnostic names the conversion function IEC would use
and then says salman does not implement it, which is the honest thing to say and is also
inconvenient. This is the largest single gap in the front end and it is a good place to
start.

## Before you open the pull request

```bash
cargo test --workspace                 # 1200 tests, ~2 s after the build
cargo fmt --all
cargo clippy --workspace --all-targets # -D warnings in CI
```

Then ask yourself the review's first question: **which test backs the claim this change
makes?** If the change added a row to `docs/CONFORMANCE.md`, the answer is a function name.
If it added a capability, the registry will check the name for you. If the answer is "it is
obvious from the code", the review will ask again.

And if you decided something the standard did not: all three steps, this pull request.
