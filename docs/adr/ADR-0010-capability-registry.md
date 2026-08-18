# ADR-0010: The capability registry as the only source of status truth

- **Status**: Accepted
- **Date**: 2026-08-18
- **Deciders**: the salman authors

## Context

Every young project publishes a status table, and every status table drifts. It is written
by hand in a README, it is optimistic on the day it is written, and it is never revisited
in the commit that makes it wrong. Nothing breaks when it goes stale, which is exactly the
problem: a document with no failure mode is a document nobody has to maintain.

The drift is worse for salman than for most projects, because salman's claims are about how
far it implements a standard and about determinism. A reader deciding whether to point this
tool at their plant is reading the status table as a safety-relevant document. "Supported"
has to mean something a stranger can check without asking.

The word that does most of the damage is "mostly". A capability that mostly works is one
that works in the cases its author tried, and the reader has no way to find out which
those were.

## Decision

Every status claim salman publishes is generated from one registry in code: `REGISTRY` in
`crates/salman-core/src/capability.rs`. Each entry carries a stable dotted id, an area, a
one-line title, a status, a milestone, a note stating the limitations a reader needs in
order not to be misled, and a list of `Evidence` — a repository-relative file path and the
exact name of a test function.

There are **four** statuses and no fifth: *implemented and tested*, *implemented,
untested*, *stub*, *planned*. "Mostly working" is not a status. Choosing between four
blunt values forces the uncomfortable judgement to be made once, in a diff, rather than
made continuously in prose.

A capability marked *implemented and tested* must cite at least one piece of evidence, and
a test walks the source tree and fails if a cited test is not there, at the named path,
spelled that way. Deleting or renaming a test therefore breaks the build of the crate that
claims it. That is the mechanism that makes the table a statement about the code rather
than a statement about someone's memory of the code.

Status markers are **shapes, not colours**: `[x]`, `[~]`, `[-]`, `[ ]`. A red/green table
that a reader with deuteranopia cannot distinguish is a defect, not a styling preference,
and a test asserts the four markers are distinct as text.

*Implemented, untested* is a first-class value and is used. `lang.st.lexer-fuzzing`
carries it today, with a note explaining that a fuzzing run demonstrates nothing was
found, which is not the same as demonstrating anything is right.

The same pattern already governs standard citations: `crates/salman-core/src/clause.rs`
holds the clause registry from which `docs/IEC_CITATIONS.md` is generated, including the
statement that IEC 61131-3:2013 (Edition 3.0) was withdrawn on 2025-05-22 and superseded
by IEC 61131-3:2025 (Edition 4.0), and that salman targets Edition 3.0 because it is the
edition its public sources let it verify.

## Consequences

Adding a feature means adding a registry entry. That is friction, and it is friction on
purpose: the moment at which someone is least inclined to write down what is *not* covered
is the moment they have just finished making something work.

The registry is a second place to update, and it can lag. Nothing detects a capability
that exists in code and is missing from the registry — only the reverse. A feature landing
without an entry is invisible to every generated document, and no test will say so.

The evidence check is a substring search for `fn <name>(` in the named file. It proves the
test exists; it cannot prove the test asserts anything. A cited test whose body is emptied
still passes this gate. It prevents citing a deleted test, not citing a vacuous one, and
this document should not imply otherwise.

Renaming a test in one crate can break the build of another. `salman-core` reads files
from across the repository, so a contributor refactoring `salman-lang` can be met with a
failure in `salman-core` whose message names their rename. The message says exactly what
to fix, which is the mitigation, but the coupling is real.

`salman-core`'s tests locate the repository root by walking up from
`CARGO_MANIFEST_DIR`, so the crate cannot be tested outside this tree. That is the same
coupling `include_str!` creates for the `VERSION` file in ADR-0008, and it has the same
consequence for publishing the crate on its own.

"Implemented, untested" appears in published documentation, and reads as an admission.
That is the intent. A project that never publishes that status is either extraordinary or
not looking.

Generation is only worth as much as its use. `render_markdown` is deterministic and tested,
and two things now consume it: `docs/STATUS.md` is committed and generated from it, and
`salman status` prints the same table for a terminal, with `--markdown` producing the
document. A test compares the committed file against what the registry renders, so the two
cannot drift. Any status table that is written by hand instead of generated is still outside
this decision's reach — the prose summaries in `README.md` and `docs/ROADMAP.md` are
hand-written and can go stale, and nothing detects it.

## Alternatives considered

**A hand-written README table.** Zero infrastructure, and it is what the project started
with. It lost for the reason at the top of this document: it has no failure mode, so it
goes stale silently and the staleness is invisible to the reader who is trusting it.

**Issue labels or a project board as the status of record.** Genuinely good at tracking
work in flight, and the place maintainers already look. It lost because it lives outside
the repository: it cannot be checked out at a tag, it is not in an offline copy, and it
cannot be tested. Someone reading salman 0.0.1 in three years needs the answer that
shipped with 0.0.1.

**Doc comments as the source of truth.** Closest to the code, and they cannot be forgotten
in quite the same way. They lost because they are not enumerable: there is no way to ask
"what does salman claim, in total" without a tool that parses them, at which point the
registry has been rebuilt in a worse notation.

**A coverage tool as the definition of tested.** Objective, automatic, and it needs no
discipline. It lost because coverage measures lines executed, not capabilities
demonstrated. A high figure across a module says nothing about whether each timer edge
case named in the standard's timing diagrams has a test of its own, which is the question
a reader is asking.

## How this is enforced

All in `crates/salman-core/src/capability.rs`:

* `tested_capabilities_must_cite_evidence` — a capability claiming *implemented and
  tested* with no evidence fails.
* `every_cited_test_exists_in_the_source_tree` — a cited test that is not there fails the
  build of this crate.
* `planned_capabilities_cite_nothing_because_nothing_exists_to_cite`.
* `capability_ids_are_unique_and_sorted` — ids are unique and the registry is kept in id
  order, so generated documents are stable.
* `every_capability_is_described_and_placed_in_a_milestone`.
* `status_markers_are_distinguishable_without_colour`.
* `rendered_status_is_deterministic`.

* `the_committed_status_document_matches_what_the_registry_renders` — `docs/STATUS.md` as
  committed must equal what the registry renders, so editing the document by hand or adding
  a capability without regenerating it fails the build.
  `crates/salman-core/src/clause.rs` has the same test for the citation document,
  `the_committed_citation_document_matches_what_the_registry_renders`.

The gap that remains is the one named under Consequences: nothing detects a capability that
exists in the code and has no registry entry. Only the reverse is checked.
