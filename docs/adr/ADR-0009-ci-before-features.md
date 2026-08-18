# ADR-0009: CI before features

- **Status**: Accepted
- **Date**: 2026-08-18
- **Deciders**: the salman authors

## Context

The usual order is code first, continuous integration when the project feels real enough
to deserve it. That order has a predictable outcome: the first CI configuration is written
against a codebase that already fails it, so the gates are loosened until they pass, and
the loosened gates are what the project keeps.

salman's engineering rules are stated in `README.md` as enforced rather than aspirational.
A rule with no job behind it is a slogan, and a project that publishes slogans as rules
has already spent the credibility it was trying to build.

There is a second, sharper problem. A workflow file is a claim about what is checked, and
a green tick is that claim being believed. A workflow named `determinism` that passes
without comparing anything is worse than having no such workflow, because it converts an
open question into a settled one in the reader's mind, at no cost in work.

## Decision

The first commit that adds source to this repository also adds
`.github/workflows/ci.yml`. There is never a moment when salman has code and no CI. The
file says so in its first two lines, and the rule it implements is rule 1 in `README.md`.

Seven workflows exist at 0.0.1: `ci`, `determinism`, `docs-links`, `fuzz`, `perf`,
`supply-chain` and `version-consistency`.

The rule this implies, and the more interesting half of the decision, is that **a workflow
must not be named after a gate it does not actually enforce**. Where the gate is not yet
possible, the workflow says so on every run — in the step name, which appears in the
checks list, and in the log.

`.github/workflows/determinism.yml` is the current case. It runs the workspace test suite
on Linux, macOS and Windows, which is a real cross-platform gate and is all it claims.
It then runs a step named
`PLACEHOLDER: trace determinism is NOT checked yet (no runtime exists)`, which emits a
warning annotation stating what ran, what did not run, and what it is blocked on. It
compares no trace, because at 0.0.1 nothing produces one.

`.github/workflows/fuzz.yml` is the same rule at a later stage, and is the evidence that
it works. When there were no fuzz targets, that workflow **failed on purpose**, with a
message naming what was missing; skipping the job would have made rule 7 false in the
quietest possible way, as an all-green checks list with the fuzzing gate silently absent
from it. The targets now exist under `fuzz/fuzz_targets/`, the job does real work, and the
guard survives in a different role: it catches `fuzz/` being deleted or emptied, because a
loop over nothing succeeds. The placeholder was filled and retired. It was not left.

## Consequences

The CI configuration is written before the thing it tests, so it is edited often in the
early life of the project — sometimes twice in a week, sometimes in the same commit as the
feature that finally gives a job something to do. That churn is the cost of the ordering
and is visible in the history.

Placeholder jobs need discipline. Each one must be removed or filled, never quietly left,
and every month one survives it becomes more furniture and less warning. The two in the
tree today are `determinism.yml`'s trace comparison and, in a data file rather than a
workflow, the `binary_size_bytes` key in `perf-budget.toml`, which stands in for an
installer budget with no installer to weigh and says so in a comment.

There is genuine duplicated runner time. `determinism.yml`'s matrix runs the same test
suite on the same three platforms as `ci.yml`'s `test` job. That overlap is accepted so
that the determinism gate has somewhere to arrive — the trace comparison needs per-OS
artefact upload and a fan-in job, a shape `ci.yml` should not grow — and it should
disappear when the comparison lands.

A green checks list is still not proof that everything ran. `fuzz.yml` is scheduled, not a
pull-request check; scheduled runs are best-effort, trigger only on the default branch,
and on a public repository are disabled automatically after sixty days without activity,
with an email to an owner rather than an announcement in the checks list. Anyone treating
the checks list as a complete account of what was verified will be wrong about fuzzing.

## Alternatives considered

**Add CI when there is something to test.** The common practice, and it has a real
argument behind it: early workflows test scaffolding, which is not where the bugs are. It
lost because "something to test" never arrives at a moment when the gates can be added
without loosening them, and because the first bug that CI would have caught arrives before
that moment.

**One workflow with every job in it.** Simpler to read, one file to edit, and no
duplicated setup steps. It lost on legibility of failure: with one workflow, every failure
reads as "CI failed", and the distinct promises — determinism, links resolve, supply
chain, version truth, the performance budget — stop being separately visible. Separate
files also mean each gate's reasoning lives in a comment block next to the gate.

**Ship the workflow only when the gate is real, and track the gap in an issue.** Tidier,
and the tracker is the conventional home for missing work. It lost because the gap then
lives outside the repository: it is not in a checkout, not in a tag, and not in front of
anyone reading a passing run. The placeholder's value is precisely that it is unavoidable.

**Mark placeholder jobs `continue-on-error`.** A neutral-to-yellow result rather than a
green tick, which is closer to honest. It lost because a permanently yellow check is a
check people learn to ignore, and because it still does not say what is missing. A named
step that prints the gap on every run does.

## How this is enforced

* `.github/workflows/ci.yml`, jobs `rustfmt`, `clippy` and `test (${{ matrix.os }})` on
  Linux, macOS and Windows.
* `.github/workflows/fuzz.yml`, step "Require fuzz targets to exist", which fails the job
  if `fuzz/` is gone or holds no target rather than passing over an empty loop.
* `.github/workflows/docs-links.yml`, job "every URL in Markdown and in Rust doc comments
  resolves", with `fail: true` stated explicitly so a future default change cannot turn it
  into a report.
* `.github/workflows/version-consistency.yml` and `.github/workflows/supply-chain.yml`,
  the latter including a step requiring every ignore and exception in `deny.toml` to state
  a reason.
* `.github/workflows/perf.yml`, which compares four measurements per platform against
  `perf-budget.toml`.

Nothing mechanical enforces the naming rule itself. There is no lint that reads a workflow
name and checks it against what the steps do, so `determinism.yml`'s honesty rests on a
comment block and a warning annotation, both of which can be deleted in the same commit
that makes the workflow start lying. If that rule is ever going to be more than an
intention, it needs a check that reads the workflow files; there is not one.
