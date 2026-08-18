# ADR-0007: Dialect as first-class configuration

- **Status**: Accepted
- **Date**: 2026-08-18
- **Deciders**: the salman authors

## Context

Every vendor's Structured Text differs from every other vendor's, and from the standard.
The differences are not exotic. They are in what is reserved, in whether a literal may
carry a sign, in whether a hexadecimal digit may be lowercase, and in what an
unparenthesised expression means.

The motivating example is `-2 ** 2`, and it is not a puzzle constructed for this document.

* IEC 61131-3:2013 Table 71 "Operators of the ST language" (Ed 3.0) puts negation, unary
  plus and `NOT` above exponentiation in binding strength, and the normative Annex A
  grammar of the same edition agrees by making the operands of `**` unary expressions.
  Under that reading `-2 ** 2` is `(-2) ** 2`, which is `4`.
* CODESYS and Beckhoff both publish binding tables in the older Edition 2 order, with
  exponentiation above negation. Under that reading `-2 ** 2` is `-(2 ** 2)`, which is
  `-4`.

Both readings are defensible and both are in service in real plants. An engineer moving
code between tools needs to know which one is being applied to their file, and needs to
know it at the moment salman objects rather than after a shift.

Edition 3.0 was withdrawn on 2025-05-22 and superseded by IEC 61131-3:2025 (Edition 4.0)
(<https://webstore.iec.ch/en/publication/4552>). salman targets Edition 3.0 because it is
the edition its public sources let it verify.

## Decision

salman models dialect differences as **data**. The `Dialect` struct in
`crates/salman-lang/src/dialect.rs` holds one field per divergence; the lexer, parser and
type checker read those fields. There is no conditional compilation and no per-vendor
build.

Two profiles exist, and `DialectId` contains exactly two variants: `Generic`, written
`generic` in a project file, and `StrictIec`, written `iec61131-3:2013-strict`. The vendor
profiles named in salman's roadmap — CODESYS, TwinCAT, Siemens SCL, Rockwell ST, OpenPLC,
Beremiz — are **not implemented**. `DialectId::from_name("codesys")` returns `None`, and a
test asserts that it does, so there is nothing to select and nothing that half-works.

Every diagnostic that depends on a dialect setting names the rule it applied.
`Dialect::rule` renders the dialect id followed by the rule and the detail, so a message
reads `iec61131-3:2013-strict: lowercase hex digits — not accepted`. That is the part of
this decision that earns its keep: someone porting a plant between vendors needs to see
not only that salman objected, but under whose rule.

On `-2 ** 2` specifically, salman implements the Edition 3 reading in both shipped
profiles, and **warns on any unparenthesised unary operand of `**`**, suggesting the
parentheses that would settle it. Nobody is silently bitten by a four-versus-minus-four
difference when their code moves.

Two settings exist because the sources contradict each other outright, rather than because
salman wanted a knob:

* `lowercase_hex_digits`. matiec restricts hexadecimal digits to uppercase and cites the
  standard for it; every vendor salman examined accepts `16#ff`. Unverified either way,
  so: permitted in `generic`, refused in strict.
* `bool_widens_to_bit_strings`. One vendor's rendering of IEC 61131-3:2013 Figure 12
  "Supported implicit type conversions" (Ed 3.0) shows `BOOL` widening to `BYTE`, `WORD`,
  `DWORD` and `LWORD`; another open implementation excludes `BOOL` from bit-string
  widening. A direct contradiction, unresolved. Permitted in `generic`, refused in strict,
  and the diagnostic says which rule it used.

A third, `signed_duration_literals`, is in the same position: matiec quotes an Edition 3
committee-draft grammar permitting `T#-5s`, while CODESYS and Beckhoff both state a sign
is not permitted.

## Consequences

Every lexer, parser and checker path that consults a setting is a branch, and a branch
needs testing in both states. Three settings differ between the two shipped profiles
today, which is tractable; the cost grows with the product of the settings, not their sum,
and a vendor profile that flips five more makes the honest test matrix considerably
larger. The differential fuzz target `fuzz/fuzz_targets/lex_differential.rs` exists partly
to spread that load.

The default is permissive, so salman accepts code the standard may not. That is a
deliberate choice — refusing real code that every vendor accepts helps nobody — and it
means a project that passes salman with default settings is not thereby a conforming
project. The strict profile exists and is one line in a project file, but it is not what a
user gets by default, and this document should not pretend otherwise.

A portability linter is now something a reader will reasonably expect: given that salman
knows the dialects differ, it ought to be able to say "this line would give `-4` under
CODESYS". It does not exist. The parser's warning on `**` is one such rule, hand-written,
and there is no general mechanism behind it.

`UnaryPowerBinding::PowerTighter` is implemented in the parser and covered by a test, but
no shipped profile selects it. That is code with no user until a vendor profile lands, and
it will rot quietly if one never does.

## Alternatives considered

**Conditional compilation, one binary per vendor.** Cheapest to write and gives the
smallest binary. It lost badly: a diagnostic cannot name a rule that was compiled out, a
user cannot switch dialect to compare two readings of their own file, and salman would
have to ship and test one artefact per vendor. The whole value here is in a single binary
that can tell you what the other tool would have done.

**One strict implementation only.** Honest and simple, and it would have made this ADR
unnecessary. It lost because it rejects large quantities of code that works today on
equipment that exists, which makes the tool unusable for the migration work it is meant
for.

**One permissive implementation only.** Accepts everything and complains about nothing.
It lost because salman would then have no way to answer the question a strict user
actually has, which is what the standard says as distinct from what the vendor allows.

**A per-vendor fork of the parser.** Occasionally the pragmatic answer when dialects
diverge grammatically rather than in details. It lost on maintenance: every fix to error
recovery, every nesting bound, every diagnostic improvement would have to be applied
several times, and the forks would drift within a release.

## How this is enforced

* `crates/salman-lang/src/dialect.rs`:
  `the_strict_dialect_differs_from_generic_on_the_unverified_points`,
  `both_dialects_follow_the_edition_3_unary_power_binding`,
  `a_dialect_rule_names_the_dialect_that_produced_it` — a rendered rule starts with the
  dialect that produced it — and `dialect_names_round_trip_case_insensitively`, which
  also asserts that an unimplemented vendor name does not resolve.
* `crates/salman-lang/src/parser.rs`:
  `unary_minus_binds_tighter_than_exponentiation_as_edition_3_orders_them`,
  `an_unparenthesised_unary_operand_of_power_is_warned_about`,
  `a_parenthesised_power_operand_does_not_warn`,
  `the_power_warning_suggests_the_parentheses_that_would_settle_it` and
  `a_dialect_that_binds_power_tighter_lifts_the_unary_back_out`.
* `crates/salman-lang/src/lexer.rs`:
  `the_strict_dialect_rejects_lowercase_hexadecimal_digits` and
  `negative_durations_are_accepted_by_the_generic_dialect_and_refused_by_the_strict_one`.
* `crates/salman-lang/src/types.rs`:
  `bool_widening_is_a_setting_because_the_sources_contradict_each_other`.
* `fuzz/fuzz_targets/lex_strict_dialect.rs` and `fuzz/fuzz_targets/lex_differential.rs`,
  run daily by `.github/workflows/fuzz.yml`.

Nothing enforces that a **new** dialect-dependent branch arrives with a test in both
states. That is discipline, and it is the first place this decision will decay.
