# Conformance

**salman claims no conformance to IEC 61131-3.** It does not claim compliance either, and
the phrase "IEC 61131-3 compliant" does not appear anywhere in this repository as a
description of salman. What salman does is aim at the Structured Text of one named edition
and publish, on this page, exactly which parts of it are implemented, which are tested,
which are absent, and which are decisions salman had to make because no source available to
it settled the question.

The edition salman aims at is **IEC 61131-3:2013 (Edition 3.0)**. That edition was
**withdrawn on 2025-05-22** and superseded by IEC 61131-3:2025 (Edition 4.0). salman targets
Edition 3.0 because it is the edition its public sources allow it to verify; citing an
edition salman cannot check would be guessing in a footnote's clothing. Edition 3.0 is
therefore never "the current standard" here, and it is never cited without its year and its
edition, because clause numbers are edition-specific: Structured Text is
IEC 61131-3:2013 §7.3 "Structured Text (ST)" (Ed 3.0), and Edition 4.0 numbers it
differently. The withdrawal date comes from the publisher's product page:
<https://webstore.iec.ch/en/publication/4552>.

No normative text of IEC 61131-3 is reproduced anywhere in this repository. Citations name a
clause, table or figure so that a reader can look it up; they do not quote it. Where a
number could not be cross-checked against a public source, `docs/IEC_CITATIONS.md` — which
is generated from the citation registry in `crates/salman-core/src/clause.rs` — says so.

The honesty standard this document tries to meet is the one set by IronPLC's edition-support
page: <https://www.ironplc.com/reference/language/edition-support.html>. Say per feature what
is there, and do not let a summary sentence do the work of a row.

---

## Status markers

| Marker | Meaning |
|---|---|
| `[x]` | implemented, and tests in this repository demonstrate it |
| `[~]` | implemented, and nothing proves it is right |
| `[-]` | a placeholder exists that says at runtime that it is a placeholder |
| `[ ]` | not written: no code, no partial behaviour, nothing to call |

These are the four values of `salman_core::capability::Status`, with the same shape markers.
Shapes rather than colours: a red/green table that some readers cannot distinguish is a
defect, not a style choice.

## The state of the whole pipeline, before any table

At 0.0.1 a Structured Text source file **is lexed, parsed, type-checked, compiled to bytecode
and executed.** `salman check`, `salman run` and `salman test` all do what their names say,
and the worked example in `examples/conveyor/` passes eight declarative tests including one
that compares a recorded trace against a committed golden file. Earlier versions of this page
said that no source file could be executed and that there was no checker and no code
generator; that was true when it was written and it is false now, and every row below has
been rewritten against the whole pipeline rather than against the parser.

That is the good half. Here is the other half, which belongs beside it rather than in a
footnote:

- **One source file per invocation.** `salman_vm::project::build` compiles one file. There is
  no project model, no multi-file compilation unit, no import and no namespace. A second file
  is not merged; it is not read.
- **Five subcommands**: `version`, `status`, `check`, `run` and `test`. There is no formatter,
  no language server, no debugger, no project file and no graphical interface.
- **Not one standard *function* is implemented.** No `*_TO_*` conversions, no `ABS`, no
  `SQRT`, no `SEL`/`MAX`/`MIN`/`LIMIT`/`MUX`, no shifts or rotates, no string functions. The
  ten standard function *blocks* are all there; the function library is not started. A
  narrowing-assignment diagnostic names the conversion function IEC would use and then says
  that salman does not implement it, which is the honest thing to say and is also
  inconvenient.
- **Three stages can refuse a program, and they refuse different things.** A construct that
  parses is not thereby compiled, and a construct the compiler refuses is refused by name.
  See *What each stage refuses*, below.
- **No construct is accepted without carrying its meaning.** There were three — a subrange
  bound, a `STRING[n]` length and `EN` — and all three are now implemented. The section that
  named them is immediately below and is kept, empty of entries, because that category is the
  one that matters most and a reader should be able to check whether it is occupied.

So a row marked `[x]` in the *statements* table now means "this form parses, is type-checked,
is compiled and runs, and a test in this repository says so". Where a row means less than
that, the row says which stage stops and what the diagnostic is called.

## Where salman accepts something and does not mean it

Nothing, at present. This section existed because three constructs were accepted by every
stage, ran without a fault, and did not carry the meaning the standard gives them. All three
are implemented, and the section is kept — empty of entries but not deleted — because the
category is the one that matters most and a reader should be able to find out whether it is
occupied.

What was here, and what each does now:

**A subrange is enforced at run time.** `Level : INT (0..100);` is a promise about what the
variable can hold, and it was previously checked only when the assigned value was a constant
the checker could see. Assigning the same 200 through a variable succeeded and the subrange
held 200. Every site that stores a value into a subrange-typed destination now emits a range
check — the full list is enumerated below, under *Every site that stores a value into a
declared destination*. A violation is a fault naming the variable, the value and the bounds —
`Level was given 200, which its declared range 0..100 excludes`.

*salman policy:* a violation is a **fault**, not a clamp. The standard does not say, and the
implementations that check at all differ. A value outside its declared range is a bug, and
continuing with a silently corrected one propagates it into whatever reads the variable next.
Tests: `crates/salman-cli/tests/constraints.rs`,
`a_subrange_bound_is_enforced_when_the_value_is_not_a_constant` and the six around it.

**A `STRING[n]` maximum length is enforced.** Assigning a longer string used to copy the whole
value, so a `STRING[4]` held ten characters and reported them. The target now receives the
leading characters that fit. Truncation rather than a fault: IEC 61131-3 defines the result of
a string assignment as the characters the target can hold, and there is no error to report.
Tests: `assigning_a_longer_string_keeps_the_characters_that_fit`,
`a_string_length_is_enforced_on_a_function_block_input`.

**`EN` and `ENO` are implemented.** IEC 61131-3:2013 Table 18 "Execution control graphically
using EN and ENO" (Ed 3.0) makes them part of the calling convention rather than something a
POU declares: `EN` decides whether the call happens at all, and `ENO` reports whether it did.
salman previously accepted a `VAR_INPUT` called `EN` and gave it no meaning, so
`F(EN := FALSE, N := 7)` called `F`.

Now: `EN` and `ENO` are available on every function and function block call without being
declared; a call with `EN` false does not happen, and does not write its inputs either,
because binding the inputs is part of the call; `ENO` is true when the call happened and false
when it did not, and is true whenever `EN` is absent; and **no POU may declare a variable
named `EN` or `ENO`** (`E0324`), because one name would then mean two things at a call site.
Using `ENO` as an input or `EN` as an output is reported by name rather than as an unknown
parameter, since `F(ENO := ok)` looks plausible and does the opposite of what it says.

*salman policy:* `EN` on a call **whose result is used** is refused (`U0301`). With `EN` false
there is no call and therefore no result, and salman will not invent one. Call it as a
statement and read the result separately. Tests: `a_call_with_enable_false_does_not_happen_at_all`,
`a_call_that_does_not_happen_does_not_write_its_inputs_either`,
`enable_out_reports_whether_the_call_happened`, `a_variable_may_not_be_called_en_or_eno`,
`enable_on_a_call_whose_result_is_used_is_refused_rather_than_invented`.

### Every site that stores a value into a declared destination

`Body::coerce` in `crates/salman-vm/src/compile.rs` describes itself as *the single place a
value becomes a value of a declared type*, and the whole argument above rests on that being
true: a promise kept at some assignment sites and not others is worse than one kept nowhere,
because the gap is invisible. The sites were therefore enumerated from the code and each was
made to prove itself. They are: an assignment statement, including one through a subscript
and one into a global; a function block input; a function block output bound out with `=>`; a
`VAR_IN_OUT` copied back, for functions and function blocks alike; a function argument, named
and positional; a function's result assigned through its own name; and the `FOR` control
variable at initialisation and at every increment. Each is covered by a test named after the
property it proves, in `crates/salman-cli/tests/constraints.rs`.

Four sites store a value and do **not** call `coerce`, and each is sound for a reason rather
than by oversight:

- **`copy_wide` and `copy_wide_from`**, which move a structure, an array or an instance slot
  by slot. The checker admits an aggregate assignment only between one type and itself — an
  aggregate has no elementary type, so nothing else is assignable to it — and salman interns
  types by structure, so "itself" includes every element bound. There is no element such a
  copy could carry that its destination's declaration excludes.
- **The `FOR` loop's limit and step**, and **the `CASE` selector**, which are salman's own
  temporaries. Nothing declares them, so they constrain nothing; the values that reach the
  control variable are checked where they reach it.
- **`ENO`**, which is written straight into its target. The checker admits nothing but a
  `BOOL` there, and a `BOOL` has no subrange to violate and no length to exceed.

Three things the audit found, and what they do now:

**A declared initial value never reaches `coerce`** — it is written into the slot before the
first scan — so it is checked in the checker instead, against the constraint its declared type
carries (`E0404`). `Level : INT (0..100) := 200;` was already refused because the literal
reports itself; `:= INT#200;`, `:= 150 + 50;` and `:= Big;` naming a `CONSTANT` were not, and
started the variable outside its own range. `Level := 200;` a line later then faulted, on a
variable already holding 200. The same check covers a global, a variable of a named subrange
type, and a `VAR` inside a function block, whose initial value belongs to every instance and
reaches memory through the layout rather than through any instruction.

*salman policy:* an initial value too long for its `STRING[n]` is **refused**, where an
assignment of one truncates. A declaration is not an assignment: it states how long the
variable is, and an initial value contradicting it is a mistake worth reporting.

**A value bound out of a call arrived with the wrong type.** The output and `VAR_IN_OUT`
copy-back paths took the type the value already had from the destination rather than from the
parameter it came out of, so `coerce` compared a type with itself, found no difference and
emitted no conversion. An `INT` output bound into a `DINT` variable left an `INT` value in a
`DINT` slot — right number, wrong type, and nothing says so until something reads it.

**A `FOR` loop over exactly the range its control variable declares could not run.** The value
that ends a `FOR` loop is one past its end by construction, so checking the incremented value
after storing it faulted on `FOR I := 0 TO 3` over `I : INT (0..3)`. The candidate is now
tested against the loop's limit before it is allowed to reach the control variable, and
checked only when the body is about to be given it.

*salman policy, and a visible consequence:* after a loop the control variable holds the last
value the body was given rather than one past it. IEC 61131-3 does not define the value of a
control variable after its loop; salman chose the one that is inside the variable's own
declared range.

Related, and part of the same defect: `BY` is a **step**, not a value of the control variable,
and it was type-checked against the control variable's declared type. That refused
`FOR I := 3 TO 0 BY -1;` over `I : INT (0..3)` — a descending loop over a non-negative
subrange could not be written at all, although every value it gives the variable is inside the
range. `BY` is now checked against the control variable's base type.

**An enumeration was a subrange in all but name, and carried none of its meaning.** An
enumeration is a base type and a **set** of legal values, and salman flattened it to the base
type and enforced nothing: `Shade := 77;` on a three-value `Colour` compiled, ran and stored
77. Both halves are closed. A value the checker can see is refused (`E0404`), and a value
arriving through a variable faults at run time naming the variable, the value and the set —
`Shade was given 77, which is not one of its declared values (0, 1, 2)`. The check is
membership rather than bounds, because the values need not be contiguous: a range check over
`(Low := 0, High := 2)` would accept the 1 that type does not have.

**One parameter could be given an argument twice.** `A(EN := TRUE, EN := FALSE)` compiled, and
the last argument won, so a reader saw a call enabled and salman skipped it. For `ENO` it was
worse: `A(ENO => First, ENO => Second)` wrote `Second` and left `First` exactly as it was, a
variable the engineer bound and nothing wrote. This was never special to `EN` and `ENO` — every
named parameter had it — so it is refused for all of them (`E0325`) rather than for the two
that made it visible.

One smaller thing, true rather than wrong, that will still surprise: **`--record` splits its
argument on commas**, so a multidimensional slot name such as `Main.G[1,1]` cannot be named on
the command line. The variable exists and holds the right value; it is reachable through a
trace of the whole program or through a declarative test.

---

## What each stage refuses

Three stages produce errors, and which stage refused a construct is worth knowing: it is the
difference between "your code is wrong", "salman has not built this yet" and "salman built it
and could not lay it out".

**The lexer and the parser** refuse text that is not Structured Text, under `E01xx` and
`E02xx`. Two of their codes are refusals rather than complaints:

- `U0101` — a literal prefix naming a type salman has not implemented: `LDATE#`, `LTOD#`,
  `LDT#`.
- `U0201` — a keyword that is reserved so that meeting one says so: `CLASS`, `METHOD`,
  `INTERFACE`, `EXTENDS`, `IMPLEMENTS`, `THIS`, `SUPER`, `REF`, `NULL`, `STEP`,
  `INITIAL_STEP`, `TRANSITION`, `ACTION`. The same code covers a structure or an enumeration
  declared inline in a variable block, and a `VAR_CONFIG` instance path.

**The checker** — `crates/salman-lang/src/sema.rs`, codes `E03xx` and `E04xx` — resolves every
name, gives every expression a type, folds constants, checks call shapes and rejects
recursion. It refuses two constructs as unimplemented rather than as wrong, both under
`U0301`, and the diagnostic says the code may well be correct and salman cannot check it:

- the dereference operator `^`
- the assignment attempt `?=`

**The compiler** — `crates/salman-vm/src/compile.rs` — runs **only when the checker reported no
error at all**, so everything it says is about a construct the checker deliberately let
through. It refuses, under `U0301`, with a message naming the construct:

| What | The message begins | Test |
|---|---|---|
| `AT %...` located variables | `salman does not implement AT %IX0.0 yet` | `diagnostics.rs: located_variables_report_that_the_io_mapping_layer_does_not_exist` |
| Exponentiation `**` | `salman does not compile exponentiation` | `diagnostics.rs: exponentiation_reports_that_it_is_not_implemented` |
| Subscripting an array whose elements occupy more than one slot | `salman does not compile subscripting an array whose elements are` | none |
| Assigning a whole aggregate through a subscript or a direct address, in either direction, including out of a function block output or a `VAR_IN_OUT` | `salman does not compile assigning a whole structure, array or function block instance` | `semantics.rs: assigning_a_whole_structure_through_a_subscript_is_refused` |
| A `VAR_EXTERNAL` declaration, which nothing binds to the global of the same name | `salman does not implement VAR_EXTERNAL Shared yet` | `semantics.rs: a_var_external_declaration_is_refused_rather_than_given_private_storage` |
| Binding an output of a `FUNCTION`, which has none; positional arguments to a function block instance | `salman does not compile binding an output of a FUNCTION` | none; the checker reaches both cases first, as `E0316` and `E0315` |

The multi-slot rule is worth stating plainly, because it is not obvious from the message: an
array is subscriptable when each element occupies exactly one slot. An `ARRAY OF DINT` and an
`ARRAY OF` a *single-field* structure are; an `ARRAY OF` a two-field structure and an
`ARRAY OF TON` are not, and neither `A[1].X` nor `A[1] := B` compiles for them. The
declaration itself is accepted and given slots, and calling `T[1](...)` is refused earlier by
the checker as `E0314`.

Where the compiler cannot resolve an expression to a place at all it reports `E0501`, "this
expression has no address salman can compute" or "this cannot be assigned to". That is a
layout gap rather than a named refusal and it is a worse diagnostic than the ones above,
because the message does not say what is missing. Today it is reachable through the same
multi-slot array cases, alongside the named refusal.

`E0501` also carries one refusal that does say what is wrong: **a function block that holds an
instance of itself**, directly or through another block, is reported as ``` `Looper` holds an
instance of itself ``` and the unit is not compiled. Such a block has no finite size, and
salman lays every instance out once, at load, so there is nowhere for the inner one to live.
`semantics.rs: a_function_block_that_holds_an_instance_of_itself_is_refused` and
`semantics.rs: two_function_blocks_that_hold_each_other_are_refused`.

`E0502` "this project has nothing to run" is the compiler's remaining error: a file with no
`PROGRAM` in it compiles to nothing schedulable.

Both the checker and the compiler spell their not-implemented refusals `U0301`. They are the
same code from two crates — `salman_lang::codes::U_REFERENCES` and
`salman_vm::compile::U_NOT_COMPILED` — and only the message distinguishes them.

---

## What is implemented

### Lexical structure

Tests are in `crates/salman-lang/src/lexer.rs` unless stated.

| Status | Feature | Evidence |
|---|---|---|
| `[x]` | Line comments `//` | `line_comments_end_at_the_line_break` |
| `[x]` | Block comments `(* *)` and `/* */` | `a_small_program_lexes_into_the_expected_tokens` |
| `[x]` | Block comments **nest** | `block_comments_nest` |
| `[x]` | Comment nesting is bounded rather than unbounded | `comment_nesting_is_bounded_so_a_hostile_file_cannot_exhaust_the_stack` |
| `[x]` | Comment spans kept, for a formatter | `comment_spans_are_recorded_for_the_formatter` |
| `[x]` | Identifiers: case-insensitive, case-preserving, ASCII case rules | `identifiers_compare_case_insensitively` (`salman-core/src/ident.rs`) |
| `[x]` | Decimal integer literals, with `_` as a digit separator | `a_misplaced_underscore_is_reported` |
| `[x]` | Based literals in radix 2, 8 and 16 | `based_literals_use_radix_2_8_and_16` |
| `[x]` | Typed literals, `INT#5`, `LREAL#1.5`, `BOOL#1` | `a_typed_literal_carries_the_type_its_prefix_named` |
| `[x]` | Real literals, exponent form, digits required both sides of the point | `real_literals_need_digits_on_both_sides_of_the_point` |
| `[x]` | `1..5` lexes as a range, not as a real followed by a real | `a_range_is_not_a_real_literal` |
| `[x]` | Duration literals `T#`/`TIME#`, units summed, descending, one fraction | `duration_literals_sum_their_units`, `duration_units_must_descend`, `only_the_last_duration_unit_may_carry_a_fraction` |
| `[x]` | Only the first unit of a duration may overflow its natural range | `the_first_unit_of_a_duration_may_overflow_but_a_later_one_may_not` |
| `[x]` | `LTIME#` marks the long form | `long_duration_prefixes_are_marked_as_ltime` |
| `[x]` | Sub-nanosecond duration truncation warns rather than silently rounding | `a_duration_finer_than_a_nanosecond_warns_that_it_was_truncated` |
| `[x]` | `D#`, `TOD#`, `DT#` literals, and a date that does not exist is refused | `date_time_and_date_and_time_literals_parse`, `a_date_that_does_not_exist_is_rejected` |
| `[x]` | `STRING` literals with the `$` escapes | `string_escapes_follow_table_7` |
| `[x]` | `WSTRING` literals with four-hex-digit escapes | `wstring_escapes_take_four_hex_digits` |
| `[x]` | Directly represented variables, hierarchical, lexed as one token | `a_hierarchical_direct_address_lexes_as_one_token` |
| `[x]` | The size letter may be omitted (`%I1` is `%IX1`) and stays omitted when printed | `the_size_letter_of_a_direct_address_is_optional` |
| `[x]` | Partly specified addresses `%I*` | `a_partly_specified_address_is_accepted` |
| `[x]` | Address depth is bounded | `address_depth_is_bounded` |
| `[x]` | Pragmas `{ ... }` recognised and skipped, never interpreted | `pragmas_are_recognised_and_skipped_without_being_interpreted` |
| `[ ]` | `CHAR` and `WCHAR` literals | — |
| `[ ]` | `LDATE#`, `LTOD#`, `LDT#` literals | refused by name: `the_long_date_types_salman_does_not_implement_say_so_plainly` |

The front end is fuzzed. Six libFuzzer targets in `fuzz/fuzz_targets` assert postconditions —
exactly one `Eof`, non-decreasing spans inside the source, every literal and address index
resolving, every node id usable as an index into a side table — rather than only that nothing
panicked. Four cover the lexer (valid UTF-8, raw bytes decoded the way the loader decodes
them, the strict dialect, and a differential run of both dialects), one covers the parser, and
one covers lexing, parsing and checking together. The capability registry records all of that
as `[~]`, not `[x]`, for two reasons that both matter: a fuzzing run shows that nothing was
found, which is not the same as showing that anything is right, and the registry's evidence
rule wants a named test function, which a libFuzzer target is not. The compiler is not fuzzed,
and neither is the declarative test-file reader in `salman-test`.

### The elementary types

Implemented, in `crates/salman-core/src/value.rs`: `BOOL`; `SINT`, `INT`, `DINT`, `LINT`;
`USINT`, `UINT`, `UDINT`, `ULINT`; `BYTE`, `WORD`, `DWORD`, `LWORD`; `REAL`, `LREAL`;
`TIME`, `LTIME`; `DATE`, `TIME_OF_DAY`, `DATE_AND_TIME`; `STRING`, `WSTRING`.

**Not implemented: `CHAR`, `WCHAR`, `LDATE`, `LTOD`, `LDT`.** They are not in
`ElementaryType`, so there is nothing to select them with and nothing that half-works.

Every one of the implemented types is a value the interpreter holds, not only a name the
parser knows. Declare a `STRING`, a `WSTRING`, a `DATE` and a `TIME`, assign to each, and
`salman run --record` prints `'hi'`, `"wide"`, `D#2024-02-29` and `T#3s`.

`D`, `DT` and `TOD` are accepted as spellings of `DATE`, `DATE_AND_TIME` and `TIME_OF_DAY`
(`elementary_type_from_word` in `salman-lang/src/token.rs`). The consequence is that none of
those three words can be used as a variable name, and the diagnostic for trying says
"expected a name, found the type `DATE`", which is accurate and initially puzzling.

| Status | Feature | Evidence (`salman-core/src/value.rs`) |
|---|---|---|
| `[x]` | Widths and ranges of the fixed-width types | `bit_widths_match_the_standard` |
| `[x]` | Default initial value of every type | `every_elementary_type_has_a_default_value_of_its_own_type`, `default_values_follow_the_iec_initial_value_table` |
| `[x]` | `STRING` holds arbitrary bytes, not UTF-8, and does not corrupt them | `strings_hold_arbitrary_bytes_without_corrupting_them` |
| `[x]` | NaN canonicalised on entry, so a trace cannot differ between architectures | `nan_is_canonicalised_so_traces_cannot_differ_between_architectures` |
| `[x]` | `-0.0` preserved, because it is portable and means something | `negative_zero_is_preserved_because_it_is_portable_and_meaningful` |

Time is modelled without leap seconds, time zones or daylight saving: every day here is
exactly 86 400 s. That is a simplification, and it is one salman states rather than
discovers later.

### The generic type hierarchy

`ANY`, `ANY_ELEMENTARY`, `ANY_MAGNITUDE`, `ANY_NUM`, `ANY_REAL`, `ANY_INT`, `ANY_SIGNED`,
`ANY_UNSIGNED`, `ANY_DURATION`, `ANY_BIT`, `ANY_CHARS`, `ANY_STRING`, `ANY_DATE` are
modelled as containment relations that the operator rules evaluate. `BOOL` is a member of
`ANY_BIT`, not a category of its own.

`[x]` — `the_generic_hierarchy_matches_the_standard_groupings`, `any_contains_every_elementary_type`,
`every_elementary_type_belongs_to_at_least_one_narrow_generic` (`salman-core/src/value.rs`).

`ANY_CHARS` currently contains exactly what `ANY_STRING` does, because `CHAR` and `WCHAR`
are not implemented.

### Implicit conversions

The governing figure is IEC 61131-3:2013 Figure 12 "Supported implicit type conversions"
(Ed 3.0). salman permits exactly these implicit conversions, written out as an adjacency
table in `crates/salman-lang/src/types.rs` rather than computed from a width rule, because
the exceptions are the interesting part:

- `REAL` → `LREAL`
- `SINT` → `INT`, `DINT`, `LINT`, `REAL`, `LREAL`
- `INT` → `DINT`, `LINT`, `REAL`, `LREAL`
- `DINT` → `LINT`, `LREAL` — **not** `REAL`, because a 24-bit significand cannot hold every
  32-bit integer
- `USINT` → `UINT`, `UDINT`, `ULINT`, `INT`, `DINT`, `LINT`, `REAL`, `LREAL`
- `UINT` → `UDINT`, `ULINT`, `DINT`, `LINT`, `REAL`, `LREAL`
- `UDINT` → `ULINT`, `LINT`, `LREAL`
- `TIME` → `LTIME`
- `BOOL` → `BYTE`, `WORD`, `DWORD`, `LWORD` — **only in the generic dialect**, see the policy
  list
- `BYTE` → `WORD`, `DWORD`, `LWORD`; `WORD` → `DWORD`, `LWORD`; `DWORD` → `LWORD`

`LREAL`, `LINT`, `ULINT`, `LTIME` and `LWORD` are the source of no implicit conversion at
all. Nothing narrows. No real converts implicitly to an integer. Numbers and bit strings do
not mix. The date, character and string types have **no** implicit conversions in salman —
and that is a stated limit of salman's source, not a claim about the figure: the
transcription is from a vendor's rendering of the conversion figure, and that rendering
omits those types entirely.

The table is no longer only data: the checker applies it to every assignment, argument and
operand, and the compiler emits a `Convert` instruction where one is needed.

| Status | Property | Evidence |
|---|---|---|
| `[x]` | `INT` widens to `REAL`, `DINT` does not | `types.rs: int_widens_to_real_but_dint_does_not` |
| `[x]` | Unsigned widens to signed only when the signed type is strictly wider | `types.rs: unsigned_widens_to_signed_only_when_the_signed_type_is_strictly_wider` |
| `[x]` | Nothing narrows implicitly | `types.rs: nothing_narrows_implicitly` |
| `[x]` | The relation has no cycle, so a common type does not depend on argument order | `types.rs: implicit_conversion_is_antisymmetric_so_there_is_no_conversion_cycle`, `types.rs: common_type_is_order_independent` |
| `[x]` | The checker applies the table to a real program, and refuses a narrowing assignment | `sema.rs: int_widens_to_real_and_dint_does_not`, `sema.rs: a_narrowing_assignment_names_the_conversion_function`, `diagnostics.rs: assigning_a_narrower_type_is_rejected` |
| `[x]` | A value of an unrelated family is refused, not coerced | `sema.rs: a_value_of_an_unrelated_type_cannot_be_assigned`, `diagnostics.rs: assigning_across_type_families_is_rejected` |
| `[x]` | Each operand keeps its own type and the operation takes the common one | `sema.rs: each_operand_keeps_its_own_type_and_the_operation_takes_the_common_one` |

The narrowing diagnostic names the IEC conversion function — `DINT_TO_INT` — and then says in
the same note that salman does not implement the standard conversion functions at v0.1, so
the only fix available today is to widen the target. That is an awkward thing for a compiler
to say and it is better than suggesting a call that does not exist.

### Operators

The precedence chain the parser implements, loosest first. Every level is left-associative.

| Level | Operators |
|---|---|
| 1 | `OR` |
| 2 | `XOR` |
| 3 | `AND`, `&` |
| 4 | `=`, `<>` |
| 5 | `<`, `>`, `<=`, `>=` |
| 6 | `+`, `-` |
| 7 | `*`, `/`, `MOD` |
| 8 | `**` |
| 9 | unary `-`, unary `+`, `NOT` |
| 10 | postfix `.`, `[]`, `^`, `()` |
| 11 | literals, names, `( )` |

| Status | Property | Evidence (`salman-lang/src/parser.rs`) |
|---|---|---|
| `[x]` | The whole chain, level by level | `or_binds_looser_than_and`, `and_binds_looser_than_equality`, `equality_binds_looser_than_the_ordering_comparisons`, `xor_sits_between_or_and_and`, `addition_binds_looser_than_multiplication`, `multiplication_binds_looser_than_exponentiation`, `mod_binds_as_tightly_as_multiplication`, `not_binds_tighter_than_equality` |
| `[x]` | Unary binds tighter than `**`, so `-2 ** 2` is `4` | `unary_minus_binds_tighter_than_exponentiation_as_edition_3_orders_them` |
| `[x]` | An unparenthesised unary operand of `**` warns | `an_unparenthesised_unary_operand_of_power_is_warned_about` |
| `[x]` | `**` groups to the left | `exponentiation_is_left_associative` — but see the UNVERIFIED list |
| `[x]` | Every level is left-associative | `every_binary_level_is_left_associative` |

Operand domains and result types are decided by `types.rs`: arithmetic on `ANY_NUM`, `MOD`
on `ANY_INT`, `AND`/`OR`/`XOR` on `ANY_BIT`, comparison across `ANY_ELEMENTARY` yielding
`BOOL`, plus duration arithmetic (`TIME + TIME`, `TIME - TIME`, `TIME * number`,
`number * TIME`, `TIME / number`; dividing a number by a duration is deliberately absent).
Negating an unsigned value promotes to the next wider signed type rather than wrapping, and
is refused for `ULINT` because nothing is wider. `[x]`, twelve tests in `types.rs`, and the
checker applies them: `sema.rs: an_operand_outside_the_operators_domain_names_the_generic_type_it_accepts`,
`diagnostics.rs: arithmetic_on_a_bit_string_is_rejected`.

Every operator in the chain is compiled and executed **except `**`**, which is where the
table above and the runtime part company:

| Status | Operator | What actually happens |
|---|---|---|
| `[x]` | `OR`, `XOR`, `AND`, `&`, `=`, `<>`, `<`, `>`, `<=`, `>=`, `+`, `-`, `*`, `/`, `MOD`, unary `-`, `NOT` | Parsed, type-checked, compiled to a `Binary` or `Unary` instruction and executed. `exec.rs: bit_operations_keep_the_width_of_their_operands`, `exec.rs: integer_division_truncates_toward_zero`, `exec.rs: strings_and_dates_compare_by_value`; `semantics.rs: an_operation_between_two_widths_is_done_at_the_wider_one`, `semantics.rs: a_bit_operation_keeps_the_width_of_its_operands`, `semantics.rs: not_inverts_every_bit_of_the_width_it_is_written_on`, `semantics.rs: unsigned_arithmetic_stays_unsigned`, `semantics.rs: a_duration_scales_by_a_number_and_compares_with_a_duration` |
| `[x]` | Unary `+` | The identity: `+X` is `X`, and it compiles to no instruction at all. `semantics.rs: unary_plus_is_the_identity_and_unary_minus_negates`, `semantics.rs: unary_plus_on_a_literal_is_the_literal` |
| `[x]` | `.` field access, `[]` subscript, `()` call | Compiled to a slot offset, a bounds-checked indexed access and a call. `exec.rs: an_array_subscript_outside_its_bounds_faults_with_the_bounds_in_the_message`; `semantics.rs: an_array_is_indexed_from_its_declared_lower_bound`, `semantics.rs: a_two_dimensional_array_is_linearised_row_by_row`, `semantics.rs: each_dimension_is_checked_against_its_own_bounds` |
| `[ ]` | `**` exponentiation | **Parses and type-checks; the compiler refuses it by name** under `U0301`, because salman implements no transcendental functions in this version. The interpreter also refuses `Pow`, but nothing compiled from source can reach that. `diagnostics.rs: exponentiation_reports_that_it_is_not_implemented` |
| `[ ]` | `^` dereference | **Parses; the checker refuses it by name** under `U0301`. There are no reference types. `sema.rs: the_dereference_operator_is_reported_as_not_implemented` |

Constant subexpressions are folded before code generation, wrapping exactly as the runtime
wraps, and a constant expression that divides by zero is an error found before the program
runs: `sema.rs: folding_wraps_the_way_the_runtime_wraps`,
`sema.rs: division_by_a_constant_zero_is_found_before_the_program_runs`. That check fires only
when the *whole* expression folds — `10 / 0` is refused, `N / 0` with `N` a variable is not,
and becomes a runtime fault instead.

### Statements

Every row below is the whole pipeline, not the parser alone: `[x]` means the form parses,
type-checks, compiles to bytecode and runs. `diagnostics.rs: every_statement_form_compiles`
puts every one of them in a single program and insists that program compiles with no error at
all; the parser tests named beside each row are what pin the *shape* the form parses into.

| Status | Statement | Evidence (`parser.rs` unless stated) |
|---|---|---|
| `[x]` | `;` — the empty statement | `a_bare_semicolon_is_the_empty_statement` |
| `[x]` | `target := value;` | `an_assignment_keeps_its_target_and_its_value`; `sema.rs: a_pou_may_assign_to_its_own_var_output_and_locals` |
| `[x]` | A call as a statement | `a_call_on_its_own_is_a_statement`; `sema.rs: a_user_function_block_is_called_through_its_instance` |
| `[x]` | `IF`/`ELSIF`/`ELSE`/`END_IF` | `if_then_end_if_has_one_branch_and_no_else`, `elsif_branches_are_kept_in_order_after_the_if`; `sema.rs: a_condition_that_is_not_bool_is_refused_and_says_why`; `semantics.rs: an_if_chain_runs_exactly_one_branch` |
| `[x]` | `CASE` with single, list and range labels, and `ELSE` | `case_labels_may_be_single_values_lists_or_ranges`, `a_case_may_have_an_else_arm`; `sema.rs: an_enumeration_selects_a_case_arm`; `semantics.rs: a_case_range_label_matches_every_value_in_it_and_none_outside_it`, `semantics.rs: a_case_selector_is_evaluated_once_and_not_again_for_each_arm`, `semantics.rs: a_case_inside_a_case_does_not_disturb_the_selector_around_it` |
| `[x]` | `FOR`/`TO`/`BY`/`DO`/`END_FOR`, including a negative step | `for_keeps_its_control_variable_bounds_and_step`, `for_without_by_records_no_step_rather_than_inventing_one`; `sema.rs: a_for_control_variable_must_be_an_integer`; `semantics.rs: a_for_loop_counts_down_when_its_step_is_negative`, `semantics.rs: a_for_loop_whose_step_overshoots_stops_at_the_limit`, `semantics.rs: a_for_loop_whose_range_is_empty_never_runs_its_body` |
| `[x]` | `WHILE` and `REPEAT` | `while_tests_before_the_body_and_repeat_tests_after_it`; `sema.rs: a_bool_condition_is_accepted_in_all_three_loops_and_in_if`; `semantics.rs: a_while_loop_that_is_false_at_entry_never_runs`, `semantics.rs: a_repeat_loop_runs_its_body_before_it_tests_and_continue_goes_to_the_test` |
| `[x]` | `CONTINUE` (new in Edition 3) | `continue_is_a_standard_statement_in_edition_3`; `sema.rs: exit_and_continue_inside_a_loop_are_accepted`; `semantics.rs: exit_leaves_the_innermost_loop_and_continue_starts_its_next_pass` |
| `[x]` | `EXIT` and `RETURN` | `exit_and_return_are_statements_of_their_own`; `sema.rs: exit_outside_a_loop_is_refused`, `diagnostics.rs: exit_outside_a_loop_is_rejected` |
| `[x]` | Calls with positional, named-input and named-output arguments, mixed | `positional_named_and_output_arguments_may_be_mixed`, `an_output_binding_with_nothing_after_it_discards_the_output`; `sema.rs: an_output_binding_writes_the_variable_it_names`; `semantics.rs: a_standard_timer_runs_from_a_program_and_binds_its_outputs`, `semantics.rs: a_function_called_twice_in_one_expression_gets_both_answers_right` |
| `[ ]` | `?=`, the assignment attempt | Parses — `an_assignment_attempt_is_parsed_rather_than_refused` — and is then **refused by the checker** under `U0301`, because salman has no reference types and so nothing for the attempt to test: `sema.rs: the_assignment_attempt_is_reported_as_not_implemented` |

The call forms are checked as well as parsed: a function block has no positional form
(`sema.rs: positional_arguments_to_a_function_block_are_refused_citing_the_call_table`), a
call may not mix positional and named arguments
(`sema.rs: a_call_may_not_mix_positional_and_named_arguments`), an unknown parameter name
lists the ones that exist
(`sema.rs: an_unknown_function_parameter_lists_the_ones_that_exist`), and a function block
call used where a value is required is refused with the dotted-notation fix
(`sema.rs: a_function_block_call_produces_no_value`).

Error recovery is a tested property, not an aspiration: `a_file_with_ten_broken_statements_reports_about_ten_errors_not_one`,
`a_broken_statement_does_not_hide_the_good_ones_after_it`, `an_error_node_never_appears_without_a_diagnostic_beside_it`.
So is the bound on nesting: `ten_thousand_nested_parentheses_produce_a_diagnostic_rather_than_a_stack_overflow`,
`a_long_operator_chain_is_bounded_too_because_its_tree_is_just_as_deep`.
Both properties survive the checker: `sema.rs: check_never_panics_on_a_unit_the_parser_could_not_finish`,
`sema.rs: a_program_with_ten_distinct_errors_reports_about_ten_diagnostics_not_one`,
`diagnostics.rs: one_broken_file_reports_many_errors_not_one`.

### Declarations, POUs and configuration

Parsed, resolved, laid out and compiled, except where a row says otherwise. Every declared
variable becomes one or more slots with a dotted name — `Main.Starter.Run_Off.ET` — which is
what makes a watch list, a trace and a force list possible without a second symbol table:
`conveyor_example.rs: the_example_declares_the_variables_the_tests_name`.

| Status | Feature | Evidence |
|---|---|---|
| `[x]` | `PROGRAM`, `FUNCTION` (with return type), `FUNCTION_BLOCK` | `parser.rs: a_function_declares_the_type_of_the_value_it_returns`, `parser.rs: a_function_block_has_no_return_type`; `diagnostics.rs: a_function_can_be_declared_and_called`, `diagnostics.rs: a_user_function_block_can_be_instantiated` |
| `[x]` | All nine `VAR` section keywords are parsed | `parser.rs: every_variable_section_keyword_opens_its_section`. Parsing a section is not implementing it; `VAR_EXTERNAL`, `VAR_ACCESS` and `VAR_CONFIG` have their own rows below |
| `[x]` | `RETAIN`, `NON_RETAIN`, `CONSTANT`, `PERSISTENT` qualifiers | `parser.rs: variable_block_qualifiers_are_recorded`; `sema.rs: a_constant_may_not_be_assigned_to`, `diagnostics.rs: writing_to_a_constant_is_rejected`; `semantics.rs: a_retained_variable_inside_a_function_block_survives_a_warm_restart` |
| `[x]` | `STRING[n]`, arrays including several dimensions, subranges, function block instances | `parser.rs: a_string_may_declare_its_maximum_length`, `parser.rs: an_array_declaration_keeps_one_dimension_per_bound_pair`, `parser.rs: a_subrange_declaration_keeps_its_base_type_and_both_bounds`, `parser.rs: a_function_block_instance_is_declared_by_naming_its_type`; `sema.rs: a_two_dimensional_array_indexes_by_both_bounds`; `diagnostics.rs: an_array_can_be_declared_indexed_and_assigned` |
| `[x]` | `TYPE` blocks: aliases, structures, enumerations, subranges, arrays | `parser.rs: a_type_block_holds_aliases_structures_enumerations_subranges_and_arrays`; `sema.rs: a_structure_field_resolves_and_an_unknown_one_does_not`, `sema.rs: enumeration_values_continue_from_the_previous_one_starting_at_zero`, `sema.rs: a_type_that_contains_itself_is_refused`; `semantics.rs: a_structure_field_and_a_global_are_reached_from_a_program_body` |
| `[x]` | Enumeration values, qualified `Colour#Green` and unqualified `Green` | `sema.rs: a_qualified_enumeration_value_resolves_to_its_number`, `sema.rs: an_unqualified_enumeration_value_resolves_from_the_type_the_context_wants`; `semantics.rs: an_unqualified_enumeration_value_compiles_and_selects_its_arm`, `semantics.rs: a_qualified_enumeration_value_means_the_same_as_an_unqualified_one` |
| `[x]` | A function block instance inside a structure, function blocks nested several deep, and a declared initial value inside one | `sema.rs: a_function_block_instance_type_knows_which_pou_declared_it`; `semantics.rs: an_instance_nested_three_blocks_deep_gets_storage_of_its_own`, `semantics.rs: a_function_block_instance_inside_a_structure_is_reached_through_the_field`, `semantics.rs: two_instances_of_one_function_block_keep_separate_state`, `semantics.rs: a_declared_initial_value_inside_a_function_block_reaches_every_instance` |
| `[x]` | A POU may be written above the blocks it instantiates | The layout iterates to a fixpoint rather than a fixed number of passes, so declaration order cannot change the answer: `semantics.rs: the_order_the_blocks_are_written_in_does_not_change_what_a_program_computes` |
| `[ ]` | A function block that holds an instance of itself, directly or through another | Refused with `E0501`, naming the block: such a block has no finite size and salman lays every instance out once. `semantics.rs: a_function_block_that_holds_an_instance_of_itself_is_refused`, `semantics.rs: two_function_blocks_that_hold_each_other_are_refused` |
| `[x]` | `VAR_GLOBAL` | `sema.rs: a_global_is_found_when_no_local_hides_it`, `sema.rs: a_local_shadows_a_global_of_the_same_name`, `sema.rs: a_configuration_global_is_visible_to_a_pou_body`; `semantics.rs: a_global_is_shared_between_two_programs_that_name_it` |
| `[ ]` | `VAR_EXTERNAL` | Parsed and resolved, and then **refused by the compiler** under `U0301`. Nothing bound it to the global of the same name: it was given storage of its own, so a POU that wrote it wrote a private copy no other POU could see. A `VAR_GLOBAL` is visible by name without the block. `semantics.rs: a_var_external_declaration_is_refused_rather_than_given_private_storage` |
| `[x]` | `CONFIGURATION`, `RESOURCE`, `TASK`, `PROGRAM ... WITH ...` | `parser.rs: a_configuration_holds_globals_resources_tasks_and_program_instances`; `sema.rs: a_configuration_produces_its_tasks_and_the_programs_bound_to_them`, `sema.rs: an_interval_that_is_not_a_positive_constant_duration_is_refused`, `sema.rs: a_single_trigger_must_name_a_global_bool` |
| `[x]` | `VAR_IN_OUT` | Passed by value at the call and copied back to the caller's variable when the call returns, which is observably the same as a reference for the forms salman compiles. There are no reference types, so it cannot be one. `semantics.rs: an_argument_written_for_a_var_in_out_parameter_reaches_it_and_comes_back` |
| `[x]` | A `FUNCTION` keeps no state between calls | Its locals start from their declared initial value on every call, which is what IEC 61131-3:2013 §6.6.2 "Functions" (Ed 3.0) makes a function mean. `semantics.rs: a_function_keeps_no_state_between_calls`, `semantics.rs: a_function_local_starts_from_its_declared_initial_value_on_every_call`, `semantics.rs: a_function_may_call_another_function` |
| `[x]` | Two instances of one `PROGRAM` keep separate state | `semantics.rs: two_instances_of_one_program_keep_separate_state` |
| `[ ]` | Enforcing a `STRING[n]` length or a subrange's bounds **at run time** | Neither is in the emitted code. Both are checked against constants at compile time, and that much is tested — `sema.rs: a_literal_outside_a_subrange_is_refused`, `sema.rs: a_string_literal_longer_than_its_target_is_refused` — but a value arriving through a variable is not checked. See the section before the tables |
| `[ ]` | `AT %IX0.0` located variables | Lexed, parsed and resolved — `parser.rs: a_located_variable_keeps_the_address_it_was_bound_to` — and then **refused by the compiler** under `U0301`, because there is no IO mapping layer to bind them to: `diagnostics.rs: located_variables_report_that_the_io_mapping_layer_does_not_exist` |
| `[x]` | Whole-aggregate assignment and argument passing: `A := B` between arrays or structures, and a structure passed to a `VAR_INPUT` | Compiled as a multi-slot copy. Not supported through a subscript or a direct address, where it is refused by name |
| `[ ]` | Arrays whose elements occupy more than one slot: an array of function block instances, or of a structure with more than one field | The declaration is accepted and given slots; subscripting one is not compiled. `T[1](...)` is refused by the checker as `E0314`, `A[1].X` and `A[1] := B` by the compiler as `U0301` and `E0501`. An array of a single-field structure occupies one slot per element and does work |
| `[ ]` | Inline structures and enumerations in a variable declaration | Named, not implemented: `parser.rs: an_inline_structure_or_enumeration_asks_for_a_named_type` |
| `[ ]` | `VAR_CONFIG` instance paths | Named: `parser.rs: an_instance_path_in_a_declaration_says_it_is_not_implemented` |
| `[ ]` | The single-resource configuration shorthand | Named in the diagnostic; tasks must sit inside a `RESOURCE` |

Recursion is rejected statically, and the whole memory layout depends on that rejection:
`sema.rs: direct_recursion_is_rejected_statically`, `sema.rs: mutual_recursion_names_the_whole_cycle`,
`sema.rs: recursion_through_a_function_block_instance_is_rejected`,
`diagnostics.rs: direct_recursion_is_rejected`, `diagnostics.rs: mutual_recursion_is_rejected`.
See the UNVERIFIED list for what salman could and could not confirm about the prohibition.

### The standard function blocks

The blocks are those of IEC 61131-3:2013 Table 43 "Standard bistable function blocks"
(Ed 3.0), Table 44 "Standard edge detection function blocks" (Ed 3.0), Table 45 "Standard
counter function blocks" (Ed 3.0) and Table 46 "Standard timer function blocks" (Ed 3.0),
with the timers' behaviour fixed by IEC 61131-3:2013 Figure 15 "Standard timer function
blocks – timing diagrams (Rules)" (Ed 3.0).

Implemented natively in `crates/salman-vm/src/stdfb.rs`, with signatures in
`crates/salman-lang/src/stdlib.rs`. None is a transcription: each is written from a
description of the observable behaviour, and every test asserts behaviour rather than body
equivalence.

| Status | Block | Evidence (`salman-vm/src/stdfb.rs`) |
|---|---|---|
| `[x]` | `SR`, set dominant | `sr_is_set_dominant_when_both_inputs_are_true`, `a_bistable_starts_reset_and_holds_its_state` |
| `[x]` | `RS`, reset dominant | `rs_is_reset_dominant_when_both_inputs_are_true`, `rs_holds_its_state_too` |
| `[x]` | `R_TRIG` | `r_trig_pulses_for_exactly_one_invocation_on_a_rising_edge`, `a_fresh_r_trig_whose_clock_is_already_true_reports_an_edge` |
| `[x]` | `F_TRIG` | `f_trig_pulses_on_a_falling_edge`, `a_fresh_f_trig_emits_one_spurious_pulse_with_its_clock_low` |
| `[x]` | `CTU` | `ctu_counts_rising_edges_only_not_levels`, `ctu_keeps_counting_past_its_preset_and_saturates_at_the_type_limit` |
| `[x]` | `CTD` | `ctd_loads_its_preset_and_load_dominates_the_count_input`, `ctd_saturates_at_the_type_minimum` |
| `[x]` | `CTUD` | `ctud_precedence_is_reset_then_load_then_counting`, `simultaneous_up_and_down_edges_leave_the_count_alone` |
| `[x]` | `TON` | `ton_does_not_fire_early`, `ton_does_not_accumulate_elapsed_time_across_separate_input_pulses` |
| `[x]` | `TOF` | `tof_holds_its_output_for_the_preset_after_its_input_falls`, `a_fresh_tof_with_its_input_low_does_not_start_an_off_delay` |
| `[x]` | `TP` | `tp_is_not_retriggerable_during_its_pulse`, `tp_is_not_truncatable_by_its_input_falling` |
| `[x]` | `SEMA` — **not a standard function block**, see the section at the end | `sema_reports_the_state_before_this_invocation_so_the_first_claimer_wins` |

The internal state of each block is an ordinary visible field (`PHASE`, `START`, `PREV_IN`,
`CU_M`) rather than hidden runtime state, because a timer whose internals you cannot watch is
a timer nobody can diagnose at three in the morning: `internal_state_is_a_visible_field_so_a_timer_can_be_debugged`.

The standard supplies **no body** for the timers — only timing diagrams — so every timer
test here is a trace of `(t, IN, PT)` against `(Q, ET)`, not a comparison against a body
salman does not have.

All eleven are reachable from Structured Text, not only from a Rust test: the checker knows
each block's parameter names and types from `stdlib.rs`, the compiler emits a native call, and
`diagnostics.rs: every_standard_function_block_can_be_declared_and_called` declares and calls
every one of the ten standard blocks from a `PROGRAM`. The worked example uses `RS`, `TOF`,
`R_TRIG`, `CTU` and `TON` together and its output is compared against a committed trace.
Writing a preset from outside the instance and reading an output back are both checked
(`sema.rs: a_timers_preset_can_be_written_from_outside`,
`sema.rs: a_timers_output_reads_as_a_member_of_its_instance`), and a block's internal field is
refused to code outside it (`sema.rs: a_blocks_internal_field_cannot_be_named_in_code`) even
though the debugger and the trace can see it.

### The scan, memory and tasks

| Status | Feature | Evidence |
|---|---|---|
| `[x]` | Inputs latched once per scan; a mid-scan change is invisible | `memory.rs: an_input_read_mid_scan_sees_the_value_it_had_at_scan_start`; `semantics.rs: an_input_read_twice_in_one_scan_reads_the_same_value_both_times`, `semantics.rs: an_input_that_changes_between_scans_is_seen_on_the_next_one` |
| `[x]` | Outputs read back within the scan, published at the end | `memory.rs: an_output_written_this_scan_reads_back_as_written_before_it_is_published`, `outputs_do_not_reach_the_world_until_the_scan_ends`; `semantics.rs: an_output_is_readable_within_the_scan_that_wrote_it_and_published_at_its_end` |
| `[x]` | A program cannot write its own `%I` | `memory.rs: a_program_cannot_write_its_own_inputs`; `exec.rs: writing_an_input_address_faults_rather_than_silently_doing_nothing` |
| `[x]` | `%M` is written through with no image | `memory.rs: marker_memory_is_written_through_with_no_image`; `semantics.rs: marker_memory_is_written_through_within_a_scan` |
| `[x]` | Bit, byte and word addresses overlay each other | `memory.rs: bit_byte_and_word_addresses_overlay_each_other_as_they_do_on_a_controller` |
| `[x]` | Force list, with the suppressed write recorded and the count never hidden | `memory.rs: a_force_records_what_the_program_wanted_so_the_difference_is_visible`, `the_force_count_is_always_available_so_no_interface_can_hide_one` |
| `[x]` | Warm and cold restart, `RETAIN` and `PERSISTENT` | `memory.rs: a_warm_restart_keeps_retain_and_persistent_and_clears_the_rest`, `a_cold_restart_keeps_only_persistent` |
| `[x]` | Cyclic tasks with a period and a priority | `task.rs: a_cyclic_task_runs_once_per_period`, `the_clock_lands_exactly_on_each_release` |
| `[x]` | Priority order, lower number first, ties broken by declaration order | `task.rs: tasks_released_together_run_in_priority_order_lower_number_first` |
| `[x]` | Event tasks released by a rising edge | `task.rs: an_event_task_runs_on_a_rising_edge_and_not_otherwise` |
| `[x]` | Freewheeling tasks | `task.rs: a_freewheeling_task_advances_the_clock_by_its_modelled_scan_time` |
| `[x]` | Overrun detection | `task.rs: a_scan_that_outlasts_its_period_is_counted_as_an_overrun` |
| `[x]` | A scan watchdog, so a runaway loop fails rather than hangs | `task.rs: the_scan_watchdog_stops_a_program_that_never_ends` |
| `[x]` | Virtual clock: monotonic, never reads a host clock, fixed epoch | `clock.rs: the_wall_clock_comes_from_a_configured_epoch_not_from_the_host`, `the_clock_never_runs_backwards` |
| `[x]` | Byte-identical trace fingerprints for the same run | `task.rs: the_same_configuration_run_twice_produces_the_same_trace_fingerprint`, `conveyor_example.rs: the_same_project_run_twice_produces_the_same_fingerprint` |
| `[x]` | Byte-identical **bytecode** from two compilations of one source file | `conveyor_example.rs: the_compiled_program_is_byte_identical_across_two_compilations` |
| `[x]` | A `%IW4` granularity setting and a process-image byte-order setting | `memory.rs: word_addressing_granularity_is_a_setting_because_vendors_disagree`, `memory.rs: image_byte_order_is_a_setting_and_round_trips_either_way`. **Both exist in the type only.** The compiler always builds memory with `ImageLayout::default()`, and no command line flag, dialect field or source construct changes either. See policies 14 and 15 |
| `[x]` | One freewheeling task per `PROGRAM` when a file declares no `CONFIGURATION` | `sema.rs: a_unit_with_no_configuration_produces_none`, `sema.rs: a_program_with_no_task_runs_freewheeling_and_is_listed_as_untasked`. A salman convenience, not a standard rule; see policy 18 |
| `[-]` | Real-time clock mode | `ClockMode::RealTime` exists, disables the determinism claim and records jitter; nothing in the tree drives it from a host clock |
| `[ ]` | Pre-emption | Not modelled at all; a scan is atomic. See policy 17 |
| `[ ]` | Mapping a located variable to the process image | The image is reachable only through a directly represented variable written out in an expression, such as `%IX0.0`. An `AT %...` binding is refused by the compiler |

The process image is fixed at 4096 bytes for each of `%I`, `%Q` and `%M`
(`compile::IMAGE_BYTES`). A real controller sizes its image from its IO configuration, which
salman will have when the IO mapping layer arrives; until then an address past the end is a
clear fault rather than a silent wrong answer
(`memory.rs: an_address_past_the_end_of_its_area_reads_none_rather_than_panicking`).

### The interpreter

`crates/salman-vm/src/exec.rs` implements the instruction set: constants, slot and address
load/store, indexed access with bounds checking, binary and unary operations, conversions,
jumps, calls, native block calls, and the instruction budget.

It now has a test module of its own — twenty-five tests — where an earlier version of this
page recorded that it had none and that everything in the file, the integer overflow policy
included, was `[~]`. That is no longer true.

| Status | Behaviour | Evidence (`salman-vm/src/exec.rs`) |
|---|---|---|
| `[x]` | Integer overflow wraps | `integer_overflow_wraps_because_that_is_what_a_controller_does`; `semantics.rs: integer_overflow_wraps_at_the_declared_width` |
| `[x]` | Integer division and remainder by zero are a fault, not a value | `integer_division_by_zero_is_a_fault_not_a_value` |
| `[x]` | The most negative integer divided by minus one does not abort the process | `the_most_negative_integer_divided_by_minus_one_does_not_abort` |
| `[x]` | Integer division truncates toward zero | `integer_division_truncates_toward_zero` |
| `[x]` | Real-to-integer conversion saturates at the target's bounds, and NaN becomes zero | `converting_a_real_to_an_integer_saturates_rather_than_being_undefined`, `converting_a_nan_to_an_integer_gives_zero` |
| `[x]` | NaN is canonicalised, and compares unequal to everything including itself | `a_nan_produced_by_the_interpreter_is_canonicalised`, `nan_compares_unequal_to_everything_including_itself` |
| `[x]` | Bit operations keep the width of their operands | `bit_operations_keep_the_width_of_their_operands` |
| `[x]` | Duration arithmetic saturates rather than wrapping | `duration_arithmetic_works_and_saturates_rather_than_wrapping` |
| `[x]` | Strings and dates compare by value | `strings_and_dates_compare_by_value` |
| `[x]` | The instruction budget stops a routine that jumps to itself | `the_watchdog_stops_a_routine_that_jumps_to_itself` |
| `[x]` | An array subscript outside its bounds faults, with the bounds in the message | `an_array_subscript_outside_its_bounds_faults_with_the_bounds_in_the_message` |
| `[x]` | A direct address reads and writes the process image; writing an input faults | `a_direct_address_reads_and_writes_the_process_image`, `writing_an_input_address_faults_rather_than_silently_doing_nothing` |
| `[x]` | Every malformed program faults rather than panicking: bad jump, missing slot, missing constant, missing routine, empty stack, unbounded stack, a condition that is not a `BOOL` | `a_jump_outside_the_routine_faults`, `a_slot_or_constant_that_does_not_exist_faults`, `a_routine_that_does_not_exist_faults_rather_than_panicking`, `popping_an_empty_stack_faults_rather_than_panicking`, `the_operand_stack_is_bounded`, `a_condition_that_is_not_a_bool_faults_rather_than_guessing` |
| `[x]` | A fault names the routine and the instruction it happened at | `a_fault_names_the_routine_and_the_instruction`; `task.rs: a_faulted_task_stops_and_the_fault_says_where` |
| `[x]` | Execution reports its instruction count, so a scan can be budgeted | `execution_reports_what_it_did_so_a_scan_can_be_budgeted` |

`crates/salman-vm/src/compile.rs` is the one large file in the workspace with **no test module
of its own**. It is covered from the outside, by the three integration files in
`crates/salman-cli/tests/`: `diagnostics.rs` for what it refuses, `semantics.rs` for what a
compiled program computes, and `conveyor_example.rs` for the whole tool. That is the right
tier for it — what a compiler owes an engineer is a correct program and a readable refusal,
not a particular instruction sequence — but it does mean no test in this repository names an
individual code-generation decision.

### The declarative test harness

`salman test <source> <tests>` runs a YAML file, or a directory of `.salman-test.yaml` files,
against a compiled program on the virtual clock. Nothing about it is IEC 61131-3; it is
recorded here because it is how a reader will check every other claim on this page.

| Status | Feature | Evidence |
|---|---|---|
| `[x]` | A test names a POU, sets `given` values, and runs `steps` that `set`, `advance`, run `scans` and `expect` | `spec.rs: a_single_test_parses`, `spec.rs: a_list_of_tests_parses`; `conveyor_example.rs: every_test_in_the_example_passes` |
| `[x]` | Values are written as IEC literals and lexed with salman's own lexer, so `T#5s` and `16#FF` mean here what they mean in source | `value.rs: every_literal_form_the_language_accepts_works_in_a_test_file`, `value.rs: a_duration_is_written_as_an_iec_literal` |
| `[x]` | An unknown key is refused rather than ignored, so a misspelled `expects:` cannot leave a test asserting nothing | `spec.rs: an_unknown_key_is_rejected_rather_than_ignored` |
| `[x]` | A skipped test must give a reason | `spec.rs: a_skipped_test_must_say_why` |
| `[x]` | `force` and `release`, so a test can hold an input against the program | `memory.rs: a_forced_slot_reads_the_forced_value_and_ignores_the_program`, `memory.rs: releasing_a_force_restores_what_the_logic_had_computed` |
| `[x]` | Golden traces: `record` a list of signals, compare against a committed text file, rewrite with `--update-golden` | `conveyor_example.rs: the_recorded_trace_matches_the_committed_golden_file`, `conveyor_example.rs: a_golden_trace_file_contains_no_carriage_returns` |
| `[x]` | A JUnit XML report and a real exit code | `report.rs: junit_output_reports_failures_and_errors_as_different_elements`, `report.rs: xml_escaping_survives_anything_an_engineer_might_type` |
| `[~]` | Each test gets a fresh copy of memory and a fresh clock, so test order cannot change a result | The code does it and is documented as doing it in `runner.rs`; **no test asserts it**, so it is `[~]` |
| `[ ]` | Any assertion about a fault, a diagnostic, or the number of scans a step took | Not expressible. A test says what variables hold, and nothing else |

`crates/salman-test/src/runner.rs` has no test module of its own; it is covered end to end
through `conveyor_example.rs`.

---

## What is NOT implemented

None of the following exists at 0.0.1. Where a keyword is reserved, meeting it produces a
message naming the construct rather than a baffling syntax error; that refusal is the whole
of the implementation.

- **Instruction List (IL).** Not implemented, not parsed, not planned as a first-class
  language.
- **Ladder Diagram (LD), Function Block Diagram (FBD), Sequential Function Chart (SFC).**
  Not implemented. `STEP`, `INITIAL_STEP`, `TRANSITION` and `ACTION` are reserved words that
  produce a named refusal.
- **The Edition 3 object-oriented extensions.** `CLASS`, `METHOD`, `INTERFACE`, `EXTENDS`,
  `IMPLEMENTS`, `THIS`, `SUPER` are reserved and produce a named refusal. There is no class
  model, no method dispatch and no interface checking.
- **References and the assignment attempt.** `REF` and `NULL` are reserved and produce a named
  refusal from the parser; the dereference `^` and the assignment attempt `?=` parse and are
  refused by the checker, under `U0301`, with a message saying the code may well be correct
  and salman cannot check it. There are no reference types. `REF_TO` is *not* reserved: a
  declaration using it gets "no type named `REF_TO` is declared", which is a worse message
  than the other four get.
- **Namespaces.** Not implemented and *not even reserved*: `NAMESPACE` is not a keyword in
  `crates/salman-lang/src/token.rs`, so a file using one gets an ordinary syntax error rather
  than a message about namespaces. That is worse than the other refusals, and it is recorded
  here rather than smoothed over.
- **The standard function library.** Not one standard *function* is implemented: no
  `*_TO_*` conversions, no `ABS`/`SQRT`/`LN`/`EXP`/trigonometry, no `SHL`/`SHR`/`ROL`/`ROR`,
  no `SEL`/`MAX`/`MIN`/`LIMIT`/`MUX`, no string functions, no time-of-day functions. Only the
  ten standard function *blocks* listed above exist, and they do work end to end. This is the
  largest single gap in the language surface: a narrowing assignment is refused with the name
  of the conversion function IEC would use, and that function does not exist to call.
- **`EN`/`ENO`.** No parsing, no checking, no execution. `EN` is not even reserved, so a POU
  may declare an ordinary `VAR_INPUT` of that name and salman will treat it as an ordinary
  input with no enable semantics. The clause is cited in the citation registry and nothing
  implements it. See the section before the tables.
- **Arrays whose elements occupy more than one slot.** The grammar accepts
  `ARRAY [1..3] OF TON` and `ARRAY [1..3] OF Point`, the checker resolves them and the
  compiler gives them slots. What does not work is reaching into one: calling `T[1](...)` is
  refused by the checker as `E0314` "only a FUNCTION and a function block instance are
  callable", and `A[1].X` and `A[1] := B` are refused by the compiler as `U0301` and `E0501`.
  An array of a *single-field* structure occupies one slot per element and works like an array
  of scalars, which is a distinction the messages do not draw.
- **`VAR_ACCESS` and `VAR_CONFIG` semantics.** The sections parse; nothing acts on them. A
  `VAR_CONFIG` instance path is refused by name.
- **A formatter, a language server, a project file, a GUI, any protocol, any network model,
  any plant model, any importer, any AI layer.** None of these has any code in this
  repository. See `docs/ROADMAP.md` for when each is intended. The declarative test harness,
  which an earlier version of this list said did not exist, does: it is `salman-test`, it is
  driven by `salman test`, and the worked example depends on it.

---

## salman policy

Each entry is a place where the standard leaves the question open, or where salman could not
verify an answer from a public source. Each says what the question is, what salman does, and
why it is a policy rather than a requirement. None of these is a claim about IEC 61131-3.

### 1. The type of an untyped literal

**Question.** `5` is an integer, but which one?

**What salman does.** An untyped literal takes the type its context requires; where there is
no context it falls back to `DINT`, and `LREAL` for a real.

**Why a policy.** No standard default could be verified from a public source. One vendor
documents `DINT`; another documents "the smallest possible type". Those give different
answers for `x : SINT := 5;` and for overload resolution, so salman picks the widely
documented pair and says it is a choice. `default_literal_type` in
`crates/salman-lang/src/types.rs`. The checker applies it and three tests cover it, where an
earlier version of this page recorded the rule as `[~]` with nothing applying it:
`sema.rs: an_untyped_integer_literal_takes_the_type_its_context_requires`,
`sema.rs: an_untyped_integer_literal_falls_back_to_dint_when_nothing_asks_for_a_type`,
`sema.rs: an_untyped_real_literal_falls_back_to_lreal`. A literal that does not fit the type
its context asks for is refused rather than wrapped:
`sema.rs: an_untyped_integer_literal_that_does_not_fit_its_context_names_the_value_and_the_range`,
`diagnostics.rs: a_literal_that_does_not_fit_its_target_is_rejected`.

### 2. Whether `BOOL` implicitly widens to the bit strings

**Question.** May `BOOL` become `BYTE`, `WORD`, `DWORD`, `LWORD` without being asked?

**What salman does.** A dialect setting. Permitted in `generic`, refused in
`iec61131-3:2013-strict`, and every diagnostic names the rule it applied.

**Why a policy.** Two sources contradict each other outright: a vendor's rendering of the
implicit-conversion figure shows `BOOL` widening, and another open implementation excludes
`BOOL` from bit-string widening. salman does not resolve the contradiction; it makes it
visible. `bool_widening_is_a_setting_because_the_sources_contradict_each_other`.

### 3. Lowercase hexadecimal digits in based literals

**Question.** Is `16#ff` legal, or only `16#FF`?

**What salman does.** A dialect setting: accepted in `generic`, refused in strict.

**Why a policy.** matiec restricts hexadecimal digits to uppercase and cites the standard for
it; every vendor salman looked at accepts lowercase. salman could not verify which is right,
and refusing real code that every tool accepts helps nobody.
`the_strict_dialect_rejects_lowercase_hexadecimal_digits`.

### 4. Signed duration literals

**Question.** Is `T#-5s` legal?

**What salman does.** A dialect setting: accepted in `generic`, refused in strict.

**Why a policy.** matiec quotes an Edition 3 committee-draft grammar that permits a sign;
CODESYS and Beckhoff both state that a sign is not permitted. Unresolved.
`negative_durations_are_accepted_by_the_generic_dialect_and_refused_by_the_strict_one`.

### 5. Unary versus exponentiation

**Question.** Is `-2 ** 2` equal to `4` or to `-4`?

**What salman does.** `4`. Unary binds tighter, following the row order of
IEC 61131-3:2013 Table 71 "Operators of the ST language" (Ed 3.0) and the Edition 3.0
normative annex grammar, in which the operands of `**` are unary expressions. salman
**warns** on any unparenthesised unary operand of `**`, so that nobody is silently bitten
when code moves between tools. A parenthesised operand does not warn.

**Why a policy.** CODESYS and Beckhoff both publish binding-strength tables in the older
Edition 2 order, with exponentiation above negation, and both give `-4` for the same text.
This is a real divergence between salman and two of the largest vendor toolchains, and it is
load-bearing: it rests entirely on the inference in the UNVERIFIED list below.

### 6. `**` associativity

**Question.** Is `2 ** 3 ** 2` equal to `64` or to `512`?

**What salman does.** `64`: `**` groups to the left, like every other binary level.

**Why a policy.** No source salman could read states the associativity of `**` *specifically*.
IEC 61131-3:2013 Table 71 "Operators of the ST language" (Ed 3.0) fixes its precedence and
says nothing about grouping, and the corresponding production in the Edition 3.0 normative
annex grammar is a repetition, conventionally read as left-associative. Three open
implementations group to the left, so salman does. This is the weakest thing in the
expression grammar, and code that depends on it should use parentheses.

### 7. `FOR` loops — three separate unknowns

No public source available to salman settles any of these.

**7a. When `TO` and `BY` are evaluated.** salman evaluates each **exactly once, at loop
entry**, and treats an absent `BY` as `1`. Evaluating them every pass would let a side effect
in the bound change the trip count part way through, which a reader of the source cannot see.
The compiler reserves two temporary slots per `FOR` statement to hold them, and
`semantics.rs: the_bounds_of_a_for_loop_are_evaluated_once_at_entry` asserts the consequence:
changing the bound variable inside the body does not change how many passes the loop makes.
The same policy applies to a `CASE` selector, which is evaluated once into a temporary:
`semantics.rs: a_case_selector_is_evaluated_once_and_not_again_for_each_arm`.

**7b. Whether the body may modify the control variable.** salman **refuses** it, with the
diagnostic saying it is a salman rule. The parser flags what it can see — a statement in the
body whose whole left-hand side is the control variable, at any nesting. It cannot see
assignment through a `VAR_IN_OUT`, through an alias, or by a callee, and it does not pretend
to; the general check belongs in the checker.
`assigning_the_control_variable_in_a_for_body_is_refused_by_a_salman_rule`.

**7c. The control variable's value after the loop.** **Unspecified by salman.** The
implementation is deterministic, and the value is deliberately not recorded anywhere in the
tree, because recording it would turn it into a promise. Code that reads it after the loop is
relying on something salman may change.

### 8. Duplicate or overlapping `CASE` labels

**Question.** Are two arms that can both match legal?

**What salman does.** Refuses them, as `E0208` and `E0209`, with the diagnostic stating that
it is a salman rule.

**Why a policy.** No source available to salman states whether the standard forbids them.
salman refuses because otherwise which arm runs would depend on the order the arms happen to
be written in, and a reader cannot see that from one arm. Only labels whose value the parser
can work out are checked; labels naming constants are compared by spelling only, and the real
check belongs in the checker.
`duplicate_case_labels_are_refused_by_a_salman_rule`, `overlapping_case_ranges_are_refused_by_a_salman_rule`.

### 9. Where `VAR` blocks may appear in a POU

**What salman does.** The parser accepts variable blocks anywhere in a POU, not only before
the first statement.

**Why a policy.** Whether the standard's grammar permits that is a question for the checker,
which can answer it with the whole POU in view. Refusing it in the parser would turn one
ordering complaint into a cascade of syntax errors.

### 10. `PT` changed while a timer is running

**Question.** What does a timer do when its preset changes mid-interval?

**What salman does.** Keeps the start instant and re-evaluates `start + PT` on every
invocation. Shortening `PT` below the elapsed time ends the interval on the next scan;
setting `PT` to zero acts as a reset; lengthening `PT` after completion resumes timing.

**Why a policy.** The standard says the effect of changing `PT` during a timing operation is
implementer-specific. It declines to define this, so salman defines it and says so. It
matches what the open and vendor implementations salman could inspect do, and common practice
is all there is to go on. `shortening_the_preset_below_the_elapsed_time_ends_the_interval`,
`lengthening_the_preset_after_completion_resumes_timing`.

### 11. A negative `PT`

**What salman does.** Refuses it as a runtime fault, naming it.

**Why a policy.** Also undefined by the standard. Negative duration literals are legal in the
generic dialect, so the parser accepts `T#-250ms`; a timer given one would otherwise produce
implementer-specific nonsense. salman prefers a fault with a name.
`a_negative_preset_is_a_fault_rather_than_an_implementer_specific_result`.

### 12. The two `TP` ambiguities

**12a. A rising edge exactly as the pulse ends.** salman's pulse-active test is strict, so
the invocation in which elapsed time reaches `PT` is already past the end of the pulse, and a
rising edge in that same invocation begins a new pulse back to back.
`a_rising_edge_in_the_invocation_a_pulse_ends_starts_the_next_one`.

**12b. When `ET` returns to zero.** salman holds `ET` at `PT` after the pulse ends and
returns it to zero only when `IN` goes low.
`tp_holds_elapsed_at_the_preset_until_its_input_goes_low`.

Both follow from timing diagrams that do not resolve the single-cycle case, so both are
salman's reading rather than a requirement salman can cite.

### 13. Integer overflow

**What salman does.** Wraps. `DINT#2147483647 + 1` is `DINT#-2147483648`.

**Why a policy.** Real controllers wrap and IEC 61131-3 does not fix the behaviour, so salman
matches hardware rather than tidiness. The wrapping policy is tested — an earlier version of
this page said it had none — by `exec.rs: integer_overflow_wraps_because_that_is_what_a_controller_does`,
and constant folding wraps identically so that a folded expression and a computed one cannot
disagree: `sema.rs: folding_wraps_the_way_the_runtime_wraps`.

### 13a. Division by zero, integer and real

**What salman does.** Integer division and remainder by zero are a **fault** that stops the
task, not a value. Real division by zero follows IEEE 754 and yields an infinity.

**Why a policy.** For the integer case there is no answer to give, and returning zero would
let a division bug reach a plant disguised as data. For the real case IEC 61131-3 references
IEEE 754 normatively for `REAL` and `LREAL`, so salman follows it rather than inventing a
second rule. `exec.rs: integer_division_by_zero_is_a_fault_not_a_value`. A division by a
constant zero is found before the program runs, when the whole expression folds:
`sema.rs: division_by_a_constant_zero_is_found_before_the_program_runs`. `N / 0` with `N` a
variable does not fold and so is a runtime fault instead.

### 13b. Converting a real to an integer saturates

**What salman does.** A `REAL` or `LREAL` converted to an integer type saturates at that
type's maximum or minimum, and a NaN becomes zero.

**Why a policy.** IEC 61131-3 does not say what an out-of-range conversion produces, and in C
it is undefined behaviour. Rust's float-to-integer cast is defined, saturating and
platform-independent, so salman takes it and says so. Going through a wider intermediate and
truncating would be worse than useless: a `REAL` of 1e30 would become whatever its low
thirty-two bits happen to be, which for that value is zero — a wrong answer that looks like a
plausible one. `exec.rs: converting_a_real_to_an_integer_saturates_rather_than_being_undefined`,
`exec.rs: converting_a_nan_to_an_integer_gives_zero`.

### 14. What `%IW4` counts

**Question.** Is `%IW4` the word at byte offset 4, or the fourth word, at byte offset 8?

**What salman does.** A setting, `AddressGranularity`, defaulting to `ElementIndex` — the
fourth word, at byte 8.

**Why a policy.** Not fixed by IEC 61131-3, and vendors genuinely differ, so the same source
text addresses different memory on different systems. Getting it wrong silently addresses the
wrong memory, which is why it is a choice rather than an assumption.
`memory.rs: word_addressing_granularity_is_a_setting_because_vendors_disagree`.

### 15. Byte order within the process image

**What salman does.** A setting, `ImageByteOrder`, defaulting to little-endian.

**Why a policy.** Also not fixed by the standard, also divergent between vendors.
`memory.rs: image_byte_order_is_a_setting_and_round_trips_either_way`.

**What is true of both 14 and 15, and needs saying.** They are settings *in the type only*.
`ImageLayout` is a field of the memory model, both alternatives work and are tested, and
nothing selects between them: `crates/salman-vm/src/compile.rs` always builds memory with
`ImageLayout::default()`, and there is no command line flag, dialect field or source construct
that changes it. So every program salman compiles today gets `ElementIndex` and little-endian.
The decision is made and recorded; the surface that would let a user choose is not built.

### 15a. The process image is a fixed 4096 bytes per area

**What salman does.** `%I`, `%Q` and `%M` are 4096 bytes each. An address past the end is a
runtime fault naming the address, not a silent wrong read.

**Why a policy.** A real controller sizes its image from its IO configuration, which salman
will have when the IO mapping layer arrives. Until then a fixed area is honest and a clear
fault is better than growing memory on demand, which would make an address typo look like it
worked. `memory.rs: an_address_past_the_end_of_its_area_reads_none_rather_than_panicking`.

### 16. Task priority ordering

**What salman does.** A **lower number is more urgent**. Ties between tasks released at the
same instant are broken by declaration order, so the answer never depends on how a collection
happened to iterate.

**Why a policy.** This is the convention across the dialect documentation salman consulted,
but the governing clause could not be verified from a public source. It is recorded as a
salman decision.
`tasks_released_together_run_in_priority_order_lower_number_first`.

### 17. Pre-emption is not modelled, and a scan is atomic

**What salman does.** A task runs to completion. A higher-priority task never interrupts a
lower-priority one part way through a scan.

**Why a policy — and a limitation, not a simplification.** Real controllers do pre-empt.
Modelling that faithfully needs an execution-cost model salman does not have. The consequence
is stated plainly: **salman cannot reproduce a race that depends on being interrupted
mid-scan.** That limitation is not hidden behind the word "deterministic", and any result
salman produces about task interaction should be read with it in mind.

### 18. What runs when a file declares no `CONFIGURATION`

**Question.** IEC 61131-3 says a configuration is what binds a program to a task. What should
a tool do with a file that has a `PROGRAM` and no configuration at all?

**What salman does.** Every `PROGRAM` in the file gets one freewheeling task of its own, in
declaration order, with priority equal to its position. A `PROGRAM` declared inside a
`CONFIGURATION` but bound to no task gets the same treatment and is listed as untasked. A
file with no `PROGRAM` at all is an error, `E0502` "this project has nothing to run".

**Why a policy.** The standard does not describe this case, because on a controller it cannot
arise. It is what makes `salman check` and `salman run` useful on a single file, and it is a
salman convenience rather than a standard rule. Anything relying on task timing should write
the configuration out. `sema.rs: a_unit_with_no_configuration_produces_none`,
`sema.rs: a_program_with_no_task_runs_freewheeling_and_is_listed_as_untasked`.

### 19. A freewheeling task has a modelled scan time

**Question.** A freewheeling task runs again as soon as it finishes. On a virtual clock, how
much time does that take?

**What salman does.** A freewheeling task is modelled as a cyclic task whose period is its own
execution time. Where no execution time is stated, `FREEWHEEL_DEFAULT_SCAN` — one millisecond
— is used.

**Why a policy.** Zero is the honest answer and it is unusable: virtual time would never
advance, so a timer inside a freewheeling program would never fire and a run would never
finish. A number had to be chosen. One millisecond is the order of magnitude of a small
program's scan; it is **not a measurement and not a claim about any controller**, and it is
why `salman run --scans 2` on a single file reports `T#1ms` rather than `T#0s`.
`task.rs: a_freewheeling_task_advances_the_clock_by_its_modelled_scan_time`.

### 20. A scan has an instruction budget

**What salman does.** Each scan may execute a bounded number of instructions. A scan that
exceeds it stops the task with a named fault — `scan used more than N instructions; salman
stopped it as a watchdog would` — rather than hanging.

**Why a policy.** `WHILE TRUE DO ; END_WHILE` must fail and say why, not wedge a test run on a
build server. Every real controller has a watchdog and this is the software equivalent; the
budget is salman's number rather than any standard's, so a program near the limit will behave
differently here from on hardware. `exec.rs: the_watchdog_stops_a_routine_that_jumps_to_itself`,
`task.rs: the_scan_watchdog_stops_a_program_that_never_ends`.

### 21. One source file per invocation

**What salman does.** `salman check`, `salman run` and `salman test` each take exactly one
Structured Text file and compile it alone. There is no project model and no multi-file
compilation unit.

**Why a policy.** Node identity is allocated per parse, so merging two parsed units means
renumbering, and that is work for a project model rather than a quiet approximation now.
salman says so rather than silently compiling only the first file and leaving a reader to
wonder where the rest went. Recorded here because "compiles Structured Text" reads as
"compiles a project" unless it is contradicted.

### 22. `SEMA` ships, and salman never calls it standard

**What salman does.** Implements `SEMA`, and returns false from `NativeBlock::is_iec_standard`
for it and for nothing else.

**Why a policy.** Existing code uses it and a tool that refuses to read the code people have
is a tool nobody can adopt; but it is in neither the Edition 2 bistable table nor
IEC 61131-3:2013 Table 43 "Standard bistable function blocks" (Ed 3.0). The full account,
including which of the two published and mutually incompatible implementations salman copies,
is in the section at the end of this page.
`stdfb.rs: sema_is_the_only_block_salman_does_not_claim_is_standard`.

### 23. What a subrange or an enumeration starts at when nothing initialises it

**What salman does.** A subrange variable with no initialiser starts at its base type's default
where its range holds that value, and otherwise at whichever declared bound is nearer it —
`low` for a range wholly above, `high` for one wholly below. `Level : INT (10..20);` therefore
starts at 10 and `Offset : INT (-20..-10);` at -10, while `Trim : INT (-5..5);` still starts at
0. An enumeration with no initialiser starts at its **first declared value**.

**Why a policy.** IEC 61131-3 gives every elementary type a default initial value and gives a
subrange no rule of its own, so the base type's default is the only value on offer — and a
subrange may exclude it. Before the bounds were enforced this was invisible. Once they are, a
variable starting outside its own declared range is indefensible: reading it and writing it
straight back faults, on a value salman itself chose. So salman keeps the standard's value
wherever the declaration permits it and changes as little as possible where it does not.
Choosing the nearer bound rather than always the lower one keeps a wholly negative range from
starting at its most negative value, which no reading of the declaration suggests.

For an enumeration the first declared value is both the widely documented rule and the only
choice that is guaranteed to be a member of the set.

`compile.rs: Compiler::declared_default`. Tests:
`a_subrange_variable_never_starts_at_a_value_its_own_declaration_excludes`,
`a_subrange_wholly_below_zero_starts_at_the_bound_nearest_zero`,
`a_subrange_that_holds_zero_still_starts_at_the_elementary_default`,
`an_enumeration_starts_at_its_first_declared_value`, and
`reading_a_subranges_initial_value_and_writing_it_straight_back_does_not_fault`, which is the
one that makes the choice mean something rather than being a number in a table.

The policy is applied in three places, because a slot acquires its initial value in three:
the load-time table for globals, program instances and function block instances; the
re-initialisation a `FUNCTION` performs on every call, since a function keeps no state; and a
function's result slot, so a function that assigns nothing does not hand its caller a value the
return type excludes.

### 24. A retained value is not re-checked when it is restored

**What salman does.** A warm restart keeps a `RETAIN` variable's value and does not test it
against the variable's declared constraint. A cold restart puts it back to the value policy 23
chose.

**Why a policy.** There is nothing to test. The only two ways a slot acquires a value are the
initial value, which policy 23 keeps inside the declaration, and a store, which goes through
`Body::coerce`. A warm restart moves no value across that boundary — it keeps one that was
already checked when it was written. Re-checking on restore would cost a pass over memory to
discover something already true, and would turn a forced value, which an engineer set
deliberately, into a fault at the next restart.
`a_retained_subrange_keeps_a_value_across_a_warm_restart_and_that_value_was_checked`.

### 25. What `STRING[n]` and `WSTRING[n]` count, and where truncation cuts

**What salman does.** `STRING[n]` is n **bytes** and `WSTRING[n]` is n **16-bit code units**,
in the checker and at run time alike, and truncation cuts at exactly n of them — through a
multi-byte UTF-8 sequence or a UTF-16 surrogate pair if that is where the cut falls.

**Why a policy.** IEC `STRING` is a sequence of single-byte characters whose encoding the
system sets, so a byte is a character and there is nothing to split. salman holds a `STRING` as
bytes and a `WSTRING` as code units precisely because it does not interpret their contents:
real projects carry values that are not valid in any encoding salman could name, and
re-encoding them on the way past would corrupt data. A truncation that decoded the value in
order to avoid splitting a sequence would be the one place that did interpret it, and would
make the length of the result depend on the data — `WSTRING[4]` holding four characters for
some values and three for others. A source literal containing a character outside ASCII
therefore occupies more than one position, and it occupies the same number of positions in the
checker as it does at run time, which is the part that has to agree.
`exec.rs: Op::TruncateString`. Tests:
`a_declared_string_length_counts_the_bytes_salman_stores`,
`truncating_a_wide_string_cuts_at_the_declared_count_even_through_a_surrogate_pair`,
`a_wide_string_is_truncated_by_code_units_not_by_bytes`,
`a_string_of_exactly_the_declared_length_is_not_truncated`, and
`a_string_with_no_declared_length_is_truncated_at_the_dialect_default`, which fixes the length
of a `STRING` written without one at the dialect's 80.

---

## UNVERIFIED

Things salman believes but could not confirm from a public source, each with what would
settle it. These are not policies — salman has no choice to defend here, only an unverified
belief it is acting on.

### That the row order of the ST operator table is the normative precedence order

salman reads the row order of IEC 61131-3:2013 Table 71 "Operators of the ST language"
(Ed 3.0) as fixing precedence, and the Edition 3.0 normative annex grammar as agreeing with
it. **Everything in policy 5 — the whole
unary-versus-exponentiation position, and therefore salman's answer of `4` for `-2 ** 2` —
rests on that inference.** If the row order is presentational rather than normative, salman
is simply wrong and CODESYS and Beckhoff are right.

*What would settle it:* reading the Edition 3.0 text of that table and of the normative
annex, or a published statement from the maintenance team about which is normative.

### `F_TRIG`'s internal memory initial value

`F_TRIG`'s internal memory has no initialiser, so it starts false, and the output is
`NOT CLK AND NOT M`. A fresh instance called with `CLK` low therefore emits one scan of `Q`
true — a falling edge that never happened. salman asserts that pulse in a test rather than
hiding it.

The whole bistable, edge-detection and counter section here is **Edition 2 text believed
unchanged in Edition 3**: the Edition 3.0 pages that define these blocks are behind a paywall
and appear in no publisher preview, so salman reconstructed the behaviour from vendor
documentation that cites both editions for identical behaviour, from open implementations,
and from a peer-reviewed formal analysis. IEC TR 61131-8 is reported to recommend the
opposite behaviour — requiring `CLK` to have been seen true first — and at least one vendor
implements the technical report instead. salman follows IEC 61131-3.

*What would settle it:* the Edition 3.0 pages for the edge-detection table.

### Whether Edition 3 counters still saturate at the counter type's limits

`CV` saturates at the counter type's own maximum and minimum, **not** at `PV`: a counter that
has reached its preset keeps counting. One widely used open implementation stops at `PV`
instead, which is a real and known disagreement. salman also does not constrain `PV` against
the type's limits, so a `PV` above the type's maximum simply never sets `Q` — salman does not
invent a constraint it cannot cite.

*What would settle it:* the Edition 3.0 counter table.

### Whether the standard states in prose that `ET` is clamped at `PT`

salman clamps `ET` at `PT`. That is read off the timing diagrams, which is the only
definition of the timers the standard gives; salman could not confirm that any prose clause
states it.

*What would settle it:* the Edition 3.0 timer clause text.

### That IEC 61131-3 forbids recursive POU invocation

salman rejects recursion statically, and **the whole memory layout depends on it**: every
slot reference in the bytecode is an absolute index, there are no stack frames and no dynamic
allocation, because every function and every function block instance is assumed to have one
permanent home in memory. That is how a real controller works and it is what makes a scan's
memory cost knowable in advance.

The prohibition is widely attested across dialect documentation. salman could not confirm the
governing clause from a public source, and the diagnostic says so.

*What would settle it:* the clause number and text of the prohibition in Edition 3.0.

---

## `SEMA` is not a standard function block

`SEMA` is in neither the Edition 2 bistable table nor IEC 61131-3:2013 Table 43 "Standard
bistable function blocks" (Ed 3.0), which between them contain every standard bistable.
salman ships it anyway, for one reason: existing code uses it, and a tool that refuses to
read the code people actually have is a tool nobody can adopt.

salman never describes it as standard. `NativeBlock::is_iec_standard` returns false for it
and only for it, and a test enforces exactly that:
`sema_is_the_only_block_salman_does_not_claim_is_standard`.

Two well-known implementations of `SEMA` disagree observably. salman copies the one that is
published verbatim with a stated rationale: `BUSY` reports the state as it was *before* this
invocation, so the first caller to claim in a scan sees `BUSY` false and wins. The other
widely used implementation has no such lag. Anyone porting between the two will see a
one-scan difference, and this paragraph is where they should find out why.

---

## How to check any of this

```
cargo test --workspace
salman check examples/conveyor/conveyor.st
salman test  examples/conveyor/conveyor.st examples/conveyor/
salman run   examples/conveyor/conveyor.st --until T#30s
salman status
```

Every test named on this page is in the file named beside it. A capability may only be
described as *implemented and tested* in `crates/salman-core/src/capability.rs` if it names
tests that exist; a test in that module fails the build if a cited test has been deleted or
renamed, and `docs/STATUS.md` is generated from that registry so the two cannot drift.

This page is written by hand and is therefore the weakest link in that chain. Three things
follow from that, and all three are worth saying plainly:

- If a row here disagrees with the code, **the code is right and this page is a bug.**
- The registry is an inventory of capabilities, not of language features. It has no entry for
  the type checker, so `docs/STATUS.md` does not list one either; this page is the only place
  the checker is accounted for, which makes this page harder to trust rather than easier.
- Nothing generates the rows below the *Status markers* table. They were written by reading
  the code and running the compiler, and every claim of the form "X is refused with diagnostic
  Y" was checked by compiling a file that does X.
