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

At 0.0.1 **no Structured Text source file can be executed by salman.** This is the single
most important fact on this page and no row below should be read without it.

- The front end lexes and parses ST into an AST. It reports errors well and recovers.
- The runtime executes bytecode, runs a scan, and implements the standard function blocks.
- **Nothing joins the two.** There is no semantic-analysis pass and no code generator.
  `crates/salman-lang/src/sema.rs` holds the data structures a checker will fill in and no
  pass that fills them; the type rules in `crates/salman-lang/src/types.rs` are written as
  data and tested as data, but nothing yet applies them to a parsed program. Every bytecode
  program in this repository was written by hand in a test.
- The `salman` binary has one subcommand: `version`.

So a row saying `[x]` in the *statements* table means "this statement form parses into the
tree it should, and a test says so". It does not mean the statement runs.

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

The lexer is fuzzed. Four libFuzzer targets in `fuzz/fuzz_targets` assert postconditions —
exactly one `Eof`, non-decreasing spans inside the source, every literal and address index
resolving — rather than only that nothing panicked. The capability registry records that as
`[~]`, not `[x]`, for two reasons that both matter: a fuzzing run shows that nothing was
found, which is not the same as showing that anything is right, and the registry's evidence
rule wants a named test function, which a libFuzzer target is not. Only the lexer is
covered; the parser is not fuzzed.

### The elementary types

Implemented, in `crates/salman-core/src/value.rs`: `BOOL`; `SINT`, `INT`, `DINT`, `LINT`;
`USINT`, `UINT`, `UDINT`, `ULINT`; `BYTE`, `WORD`, `DWORD`, `LWORD`; `REAL`, `LREAL`;
`TIME`, `LTIME`; `DATE`, `TIME_OF_DAY`, `DATE_AND_TIME`; `STRING`, `WSTRING`.

**Not implemented: `CHAR`, `WCHAR`, `LDATE`, `LTOD`, `LDT`.** They are not in
`ElementaryType`, so there is nothing to select them with and nothing that half-works.

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

| Status | Property | Evidence (`salman-lang/src/types.rs`) |
|---|---|---|
| `[x]` | `INT` widens to `REAL`, `DINT` does not | `int_widens_to_real_but_dint_does_not` |
| `[x]` | Unsigned widens to signed only when the signed type is strictly wider | `unsigned_widens_to_signed_only_when_the_signed_type_is_strictly_wider` |
| `[x]` | Nothing narrows implicitly | `nothing_narrows_implicitly` |
| `[x]` | The relation has no cycle, so a common type does not depend on argument order | `implicit_conversion_is_antisymmetric_so_there_is_no_conversion_cycle`, `common_type_is_order_independent` |

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
is refused for `ULINT` because nothing is wider. `[x]`, twelve tests in `types.rs`.

### Statements

Parsed into the tree, with the tests in `crates/salman-lang/src/parser.rs`. Nothing executes
them; see the pipeline note above.

| Status | Statement | Evidence |
|---|---|---|
| `[x]` | `;` — the empty statement | `a_bare_semicolon_is_the_empty_statement` |
| `[x]` | `target := value;` | `an_assignment_keeps_its_target_and_its_value` |
| `[x]` | A call as a statement | `a_call_on_its_own_is_a_statement` |
| `[x]` | `IF`/`ELSIF`/`ELSE`/`END_IF` | `if_then_end_if_has_one_branch_and_no_else`, `elsif_branches_are_kept_in_order_after_the_if` |
| `[x]` | `CASE` with single, list and range labels, and `ELSE` | `case_labels_may_be_single_values_lists_or_ranges`, `a_case_may_have_an_else_arm` |
| `[x]` | `FOR`/`TO`/`BY`/`DO`/`END_FOR` | `for_keeps_its_control_variable_bounds_and_step`, `for_without_by_records_no_step_rather_than_inventing_one` |
| `[x]` | `WHILE` and `REPEAT` | `while_tests_before_the_body_and_repeat_tests_after_it` |
| `[x]` | `CONTINUE` (new in Edition 3) | `continue_is_a_standard_statement_in_edition_3` |
| `[x]` | `EXIT` and `RETURN` | `exit_and_return_are_statements_of_their_own` |
| `[x]` | Calls with positional, named-input and named-output arguments, mixed | `positional_named_and_output_arguments_may_be_mixed`, `an_output_binding_with_nothing_after_it_discards_the_output` |
| `[ ]` | `?=`, the assignment attempt | parsed so it can be named: `an_assignment_attempt_is_parsed_rather_than_refused` |

Error recovery is a tested property, not an aspiration: `a_file_with_ten_broken_statements_reports_about_ten_errors_not_one`,
`a_broken_statement_does_not_hide_the_good_ones_after_it`, `an_error_node_never_appears_without_a_diagnostic_beside_it`.
So is the bound on nesting: `ten_thousand_nested_parentheses_produce_a_diagnostic_rather_than_a_stack_overflow`,
`a_long_operator_chain_is_bounded_too_because_its_tree_is_just_as_deep`.

### Declarations, POUs and configuration

All parsed; none resolved, checked or compiled.

| Status | Feature | Evidence (`salman-lang/src/parser.rs`) |
|---|---|---|
| `[x]` | `PROGRAM`, `FUNCTION` (with return type), `FUNCTION_BLOCK` | `a_function_declares_the_type_of_the_value_it_returns`, `a_function_block_has_no_return_type` |
| `[x]` | All nine `VAR` sections | `every_variable_section_keyword_opens_its_section` |
| `[x]` | `RETAIN`, `NON_RETAIN`, `CONSTANT`, `PERSISTENT` qualifiers | `variable_block_qualifiers_are_recorded` |
| `[x]` | `AT %IX0.0` located variables | `a_located_variable_keeps_the_address_it_was_bound_to` |
| `[x]` | `STRING[n]`, arrays, subranges, function block instances | `a_string_may_declare_its_maximum_length`, `an_array_declaration_keeps_one_dimension_per_bound_pair`, `a_subrange_declaration_keeps_its_base_type_and_both_bounds`, `a_function_block_instance_is_declared_by_naming_its_type` |
| `[x]` | `TYPE` blocks: aliases, structures, enumerations, subranges, arrays | `a_type_block_holds_aliases_structures_enumerations_subranges_and_arrays` |
| `[x]` | `CONFIGURATION`, `RESOURCE`, `TASK`, `PROGRAM ... WITH ...` | `a_configuration_holds_globals_resources_tasks_and_program_instances` |
| `[ ]` | Inline structures and enumerations in a variable declaration | named, not implemented: `an_inline_structure_or_enumeration_asks_for_a_named_type` |
| `[ ]` | `VAR_CONFIG` instance paths | named: `an_instance_path_in_a_declaration_says_it_is_not_implemented` |
| `[ ]` | The single-resource configuration shorthand | named in the diagnostic; tasks must sit inside a `RESOURCE` |

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

### The scan, memory and tasks

| Status | Feature | Evidence |
|---|---|---|
| `[x]` | Inputs latched once per scan; a mid-scan change is invisible | `memory.rs: an_input_read_mid_scan_sees_the_value_it_had_at_scan_start` |
| `[x]` | Outputs read back within the scan, published at the end | `memory.rs: an_output_written_this_scan_reads_back_as_written_before_it_is_published`, `outputs_do_not_reach_the_world_until_the_scan_ends` |
| `[x]` | A program cannot write its own `%I` | `memory.rs: a_program_cannot_write_its_own_inputs` |
| `[x]` | `%M` is written through with no image | `memory.rs: marker_memory_is_written_through_with_no_image` |
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
| `[x]` | Byte-identical trace fingerprints for the same run | `task.rs: the_same_configuration_run_twice_produces_the_same_trace_fingerprint` |
| `[-]` | Real-time clock mode | `ClockMode::RealTime` exists, disables the determinism claim and records jitter; nothing in the tree drives it from a host clock |
| `[ ]` | Pre-emption | not modelled at all; see the policy list |

### The interpreter

`crates/salman-vm/src/exec.rs` implements the instruction set: constants, slot and address
load/store, indexed access with bounds checking, binary and unary operations, conversions,
jumps, calls, native block calls, and the instruction budget.

**It has no test module of its own.** Two of its behaviours are covered indirectly through
`task.rs` — integer division by zero as a fault (`a_faulted_task_stops_and_the_fault_says_where`)
and the watchdog (`the_scan_watchdog_stops_a_program_that_never_ends`) — and the standard
function blocks exercise memory access. Everything else in that file, including the integer
overflow policy stated below, is `[~]`: implemented, and nothing proves it is right.

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
- **References and the assignment attempt.** `REF`, `NULL`, the dereference `^` and `?=` are
  parsed far enough to be named and are refused. There are no reference types.
- **Namespaces.** Not implemented and *not even reserved*: `NAMESPACE` is not a keyword in
  `crates/salman-lang/src/token.rs`, so a file using one gets an ordinary syntax error rather
  than a message about namespaces. That is worse than the other refusals, and it is recorded
  here rather than smoothed over.
- **The standard function library.** Not one standard *function* is implemented: no
  `*_TO_*` conversions, no `ABS`/`SQRT`/`LN`/`EXP`/trigonometry, no `SHL`/`SHR`/`ROL`/`ROR`,
  no `SEL`/`MAX`/`MIN`/`LIMIT`/`MUX`, no string functions, no time-of-day functions. Only the
  ten standard function *blocks* listed above exist. The interpreter additionally refuses
  `**` at run time, because salman implements no transcendental functions in this version.
- **`EN`/`ENO`.** No parsing, no checking, no execution. The clause is cited in the citation
  registry and nothing implements it.
- **Arrays of function block instances.** The grammar accepts `ARRAY [1..3] OF TON` because
  any named type may be an element type, and nothing resolves it, allocates it or calls it,
  so it has no meaning at 0.0.1. It is neither supported nor diagnosed — the worst of the
  three states, and it will be fixed by the checker.
- **`VAR_ACCESS` and `VAR_CONFIG` semantics.** The sections parse; nothing acts on them.
- **Semantic analysis.** `sema.rs` has the data structures and no pass.
- **Code generation.** Nothing turns an AST into bytecode.
- **A test harness, a formatter, a language server, a project file, a GUI, any protocol, any
  network model, any plant model, any importer, any AI layer.** None of these has any code
  in this repository. See `docs/ROADMAP.md` for when each is intended.

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
`crates/salman-lang/src/types.rs`. **This rule is `[~]`: the function exists, nothing applies
it yet, and no test covers it.**

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
normative annex grammar, in which the operands of `**` are unary expressions. salman **warns** on any unparenthesised unary operand of `**`, so that nobody is
silently bitten when code moves between tools. A parenthesised operand does not warn.

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
annex grammar is a repetition, conventionally read as left-associative. Three open implementations group to
the left, so salman does. This is the weakest thing in the expression grammar and code that
depends on it should use parentheses.

### 7. `FOR` loops — three separate unknowns

No public source available to salman settles any of these.

**7a. When `TO` and `BY` are evaluated.** salman evaluates each **exactly once, at loop
entry**, and treats an absent `BY` as `1`. Evaluating them every pass would let a side effect
in the bound change the trip count part way through, which a reader of the source cannot see.

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
matches hardware rather than tidiness. Integer division and remainder by zero are faults
rather than values, because there is no answer to give and returning zero would let a
division bug reach a plant disguised as data. Real division by zero follows IEEE 754 and
yields an infinity, because IEC 61131-3 references IEEE 754 normatively for `REAL` and
`LREAL`. **The wrapping policy has no test.** It is `[~]`.

### 14. What `%IW4` counts

**Question.** Is `%IW4` the word at byte offset 4, or the fourth word, at byte offset 8?

**What salman does.** A setting, `AddressGranularity`, defaulting to `ElementIndex` — the
fourth word, at byte 8.

**Why a policy.** Not fixed by IEC 61131-3, and vendors genuinely differ, so the same source
text addresses different memory on different systems. Getting it wrong silently addresses the
wrong memory, which is why it is a choice rather than an assumption.
`word_addressing_granularity_is_a_setting_because_vendors_disagree`.

### 15. Byte order within the process image

**What salman does.** A setting, `ImageByteOrder`, defaulting to little-endian.

**Why a policy.** Also not fixed by the standard, also divergent between vendors.
`image_byte_order_is_a_setting_and_round_trips_either_way`.

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
bistable function blocks" (Ed 3.0), which between them contain every standard bistable. salman ships it anyway, for one reason: existing code uses it, and a
tool that refuses to read the code people actually have is a tool nobody can adopt.

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
```

Every test named on this page is in the file named beside it. A capability may only be
described as *implemented and tested* in `crates/salman-core/src/capability.rs` if it names
tests that exist; a test in that module fails the build if a cited test has been deleted or
renamed. This page is written by hand and is therefore the weakest link in that chain: if a
row here disagrees with the code, the code is right and this page is a bug.
