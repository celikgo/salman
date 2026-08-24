---
name: citing-the-standard
description: salman's IEC 61131-3 citation policy — numbers and titles as references only, never the normative text, always with the year and edition because clause numbers are edition-specific — plus the ClauseRef registry in salman-core that generates docs/IEC_CITATIONS.md and the tests that stop a citation being decoration. This is a legal constraint as much as a stylistic one. Use whenever writing a clause, table or figure number anywhere in the tree (code comments, tests, docs), when adding an entry to clause.rs, when the IEC_CITATIONS drift test fails, or when tempted to write "IEC 61131-3 compliant".
---

# Citing IEC 61131-3

Five modules in `salman-lang` open with the same citation paragraph — `ast.rs`, `parser.rs`,
`sema.rs`, `stdlib.rs` and `types.rs`. `lexer.rs` carries a longer variant of it.
`crates/salman-lang/src/types.rs` is the canonical copy:

```rust
//! Citation policy: salman cites IEC 61131-3:2013 (Edition 3.0) clause, table
//! and figure numbers as references only. No IEC text is reproduced. Clause
//! numbers are edition-specific; see `docs/IEC_CITATIONS.md`.
```

That is a **legal** position, not a house style, and `LEGAL.md` §2 spells out how far the
confidence extends and where it stops. A contributor will not infer any of it, so this skill
states it plainly.

## The four rules

1. **Numbers and printed titles only.** A clause number is a locator. It tells a reader where
   to look and conveys none of what the standard says.
2. **Never reproduce normative content**, and that includes close paraphrase. `LEGAL.md` §2
   is explicit: *"Never transcribe the standard's sentence and swap words for synonyms: a
   close paraphrase is functionally a reproduction, and dressing a reproduction as a summary
   makes it worse rather than better."* No exception for short extracts, and no exception for
   code — function block bodies, tables, figures, grammar productions and annex examples are
   all normative content.
3. **Always carry the year and the edition.** Clause numbers renumber between editions:
   Structured Text is §7.3 in Edition 3.0 and §7.2 in Edition 4.0, and the standard function
   block tables move from 43–46 to 44–47. A number without an edition identifies nothing.
   The house form is `IEC 61131-3:2013 §7.3.3 "Statements" (Ed 3.0)`.
4. **salman claims no conformance and no compliance.** The phrase *"IEC 61131-3 compliant"*
   does not appear anywhere in this repository as a description of salman, and `LEGAL.md` §2
   lists three convenient arguments that are forbidden: that the IEC permits citation of
   clause numbers (their policy is silent, and silence is not permission), that short extracts
   are fair use, and that *ASTM v. Public.Resource.Org* covers salman (it does not).

Nothing in CI greps for the forbidden phrases. Rules 1–4 are held up by review and by the
registry below.

### Why Edition 3.0, which is withdrawn

IEC 61131-3:2013 (Edition 3.0) was **withdrawn on 2025-05-22** and superseded by
IEC 61131-3:2025 (Edition 4.0). salman targets Edition 3.0 anyway, because it is the edition
its public sources allow it to verify — targeting one it cannot check would be guessing in a
footnote's clothing. Edition 3.0 is therefore never called "the current standard" here.
`LEGAL.md` §3 and the header of `docs/IEC_CITATIONS.md` both say this; keep them agreeing.

## The registry

`crates/salman-core/src/clause.rs` holds every `ClauseRef` salman attaches to a diagnostic or
a test — 43 entries in `REGISTRY` at the time of writing — and `docs/IEC_CITATIONS.md` is
generated from it, so adding a citation to a test and forgetting to document it is not
possible. Clause and table numbers also appear in prose comments that are not registry
entries; the rules above apply to those too, but only the registry is mechanically checked.

A citation is a `ClauseRef`:

| Field | What it is |
|---|---|
| `standard` | `"IEC 61131-3:2013"` — with the year, which is not decoration |
| `edition` | `"3.0"` |
| `kind` | `CitationKind::Clause`, `Table` or `Figure` |
| `number` | `"6.6.2"` or `"71"`, bare — the `§` / `Table ` / `Figure ` prefix comes from `kind` |
| `title` | The title as printed in the contents, list of tables or list of figures |
| `requirement` | A paraphrase **in salman's own words** of the requirement being tested. Never the normative text. |
| `provenance` | `Provenance::PublicSource(url)` or `Provenance::NumberUnconfirmed` |
| `tests` | `&[CitedTest { file, test }]`. Never empty. |

Three numbering schemes run in parallel in a standard and they are not interchangeable:
§6.4.2, Table 10 and Figure 12 are three different places. That is why `kind` exists rather
than one `number` field — rendering all of them as `§10` sends a reader to a clause that has
nothing to do with the claim.

Entries are built with the private `clause()`, `table()` and `figure()` constructors, which
fill in the standard, the edition and the kind, so those three cannot be pasted wrong.

### Provenance is the honest part

`Provenance::NumberUnconfirmed` means: **trust the title, not the number.** The behaviour is
well attested across dialect documentation and open implementations, but the clause *number*
could not be confirmed from a public source. `ClauseRef`'s `Display` appends
`[clause number unconfirmed]` so a reader cannot miss it, and
`citation_display_flags_unconfirmed_numbers_so_a_reader_cannot_miss_it` asserts that.

The standard is paywalled. Numbers that *are* confirmed were cross-checked against the
publisher's own front-matter preview (which contains the contents and the lists of tables and
figures) and against vendor implementer compliance statements (which enumerate the feature
tables). `confirmed_citations_carry_a_resolvable_url` requires a URL on every confirmed
entry, and the `docs-links` workflow then checks that the URL actually resolves.

## The tests that stop a citation being decoration

All in `crates/salman-core/src/clause.rs`. One of them —
`every_cited_test_exists_in_the_source_tree` — reads files outside its own crate, so it can
fail because of a rename you made somewhere else. The rest read only `REGISTRY`,
`render_markdown()` and the committed document.

| Test | What it refuses |
|---|---|
| `every_citation_names_at_least_one_test` | A citation with an empty `tests` list. Retire a clause by deleting the row, never by leaving it standing with nothing behind it. |
| `every_cited_test_exists_in_the_source_tree` | A `CitedTest` whose file does not contain `fn <name>(`. **Renaming a test in `salman-lang` can break this test in `salman-core`.** |
| `every_citation_names_a_standard_edition_number_and_title` | An entry missing any of those. |
| `every_citation_carries_the_year_so_a_number_identifies_one_document` | A `standard` string without a year. |
| `every_citation_paraphrases_the_requirement_it_tests` | An empty or absent `requirement`. |
| `confirmed_citations_carry_a_resolvable_url` | `PublicSource` with nothing to resolve. |
| `no_clause_number_goes_deeper_than_the_three_levels_the_contents_publish` | A fourth-level number, which would be invented — the published contents stop at three. |
| `a_kind_and_a_number_together_identify_exactly_one_entry` | Two entries claiming the same place. |
| `tables_and_figures_are_not_rendered_as_clauses` | The `§10` / `Table 10` confusion above. |
| `rendered_markdown_is_deterministic` | Rendering that depends on anything but source order. |
| `rendered_markdown_says_the_cited_edition_is_withdrawn` | A generated document that quietly drops the withdrawal notice. |
| `the_committed_citation_document_matches_what_the_registry_renders` | `docs/IEC_CITATIONS.md` drifting from `REGISTRY`. |

## Adding a citation

1. **Write the test first.** A citation is a claim that salman implements what the clause
   requires, and the test is the evidence. Name it as a sentence — that name goes in the
   generated document and is what a reader sees.
2. **Add the `ClauseRef` const** in `clause.rs`, in the right numeric position: the `REGISTRY`
   order is clauses by number, then tables by number, then figures by number, and it is
   emitted verbatim, so a misplaced entry produces a spurious diff for ever.

   ```rust
   /// What this clause is about, in one line.
   pub const SOMETHING: ClauseRef = table(
       "71",
       "Operators of the ST language",
       "The precedence and associativity salman's expression parser implements",
       PREVIEW,                       // or SIEMENS, or Provenance::NumberUnconfirmed
       &[CitedTest {
           file: "crates/salman-lang/src/parser.rs",
           test: "multiplication_binds_looser_than_exponentiation",
       }],
   );
   ```

   That test name is a real one — `crates/salman-lang/src/parser.rs` — because a made-up name
   in an example is a name somebody will paste, and
   `every_cited_test_exists_in_the_source_tree` would then fail in a crate they were not
   editing.

   Use `Provenance::NumberUnconfirmed` unless you actually opened a public source and read the
   number. There is no penalty for saying so; there is a large one for a confident wrong
   number in a document whose whole purpose is provenance.
3. **Add it to `REGISTRY`.**
4. **Regenerate `docs/IEC_CITATIONS.md`:**

   ```bash
   SALMAN_UPDATE_GOLDEN=1 cargo test -p salman-core \
       the_committed_citation_document_matches_what_the_registry_renders
   ```

   The same gesture regenerates `docs/PLCOPEN_COMPATIBILITY.md` and the analyser's golden
   reports, so it is one thing to learn rather than three.
   `the_committed_citation_document_matches_what_the_registry_renders` is both the writer and
   the drift check: with the variable set it writes the file and returns, without it it
   compares, and its failure message names the command.

   **Read the diff before you commit it.** A citation registry that regenerates without
   anybody looking is a registry nobody is checking, which is the failure mode the whole
   provenance apparatus exists to prevent.
5. **Never edit `docs/IEC_CITATIONS.md` by hand.** Its own header says so, and the drift test
   will catch you.

## Citing from a diagnostic

The point of the registry is that a citation reaches the user, not just the doc. Attach one
with `with_clause`:

```rust
Diagnostic::error(codes::E_TYPE_MISMATCH, "…")
    .with_primary(span, "…")
    .with_clause(clause::FIGURE_IMPLICIT_CONVERSIONS)
```

which renders as the `= standard:` and `= requirement:` lines beneath the caret:

```
  = standard: IEC 61131-3:2013 Figure 12 "Supported implicit type conversions" (Ed 3.0)
  = requirement: The graph of conversions a conforming implementation performs without being asked, which is the set salman's type checker must not widen
```

Note what the user sees: a locator and salman's own paraphrase. Not one word of the standard.

## When you cannot verify a number at all

Two different situations, two different homes, and mixing them up is the mistake to avoid:

- **salman had to choose** between readings the standard leaves open → it is a *policy*.
  `docs/CONFORMANCE.md` `## salman policy`, plus a `salman policy` marker in the source and a
  named test. See the `extending-structured-text` skill.
- **salman believes something it could not confirm** and has no choice to defend →
  `docs/CONFORMANCE.md` `## UNVERIFIED`, with a *"What would settle it"* line saying exactly
  what document or statement would resolve it.

`UNVERIFIED` is where the most load-bearing uncertainty in the whole front end lives: salman
reads the row order of Table 71 as fixing operator precedence, and *everything* about the
unary-versus-exponentiation position — salman's answer of `4` for `-2 ** 2`, against CODESYS
and Beckhoff's `-4` — rests on that inference. The `UNVERIFIED` entry says so in those words.
That is the standard of honesty to match.

## The one-line test

Before you commit a sentence containing a clause number, ask: *could a reader who owns the
standard use this to check salman's work, without this document having told them what the
standard says?* If yes, it is a citation. If the sentence would still be useful to someone
who does not own the standard, you have probably reproduced something.
