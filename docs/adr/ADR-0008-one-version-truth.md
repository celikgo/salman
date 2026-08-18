# ADR-0008: One source of version truth

- **Status**: Accepted
- **Date**: 2026-08-18
- **Deciders**: the salman authors

## Context

The version number of this project is written down in more places than anyone remembers
while making a release. At 0.0.1 it appears as `[workspace.package] version`, three times
as a version requirement in `[workspace.dependencies]`, in the root `VERSION` file, and in
what `salman version` prints for a user.

Version skew is a quiet failure. A binary that reports one number while its release notes
report another is not a build error; it is a support conversation six months later, where
nobody can establish which build a plant is actually running. For a tool that expects to
be pointed at industrial equipment, "which version is this" has to have exactly one
answer.

The obvious mitigations are all mitigations of the wrong kind: a checklist, a release
script, a reviewer who notices. Each works until the once it does not.

## Decision

The root `VERSION` file is authoritative. `crates/salman-core/src/version.rs` embeds it
with `include_str!` and asserts, in a `const` block, that its contents equal
`CARGO_PKG_VERSION`. The two therefore cannot drift on any machine: a mismatch is a build
failure everywhere, for everyone, not a CI job that someone might skip, disable or fail to
run on a branch.

Specifics. `VERSION::VERSION` is the trimmed contents of the file, and everything that
wants the version reads it from there — `salman_core::VERSION` is what the CLI prints and
what clap reports for `--version`. Two small `const fn`s exist because `str::trim_end` and
string equality are not usable in a `const` context, and they are the whole of the
machinery.

`.github/workflows/version-consistency.yml` is the cross-check over what the compiler
cannot see. The `const` assertion sees only the version of the crate being compiled; it
cannot see the resolved version of every workspace member, it cannot see the literal
`version = "0.0.1"` written next to a path dependency in `[workspace.dependencies]` — a
path dependency wins over the requirement beside it, so the workspace would build happily
with the two disagreeing — and it cannot see what the shipped binary prints. That job
covers those three.

## Consequences

Bumping the version means editing the `VERSION` file and the number in `Cargo.toml`. This
decision does not reduce the number of places the version is written; it removes the
possibility of them disagreeing. Saying "edit one file" would be tidier and would be
false: the compiler makes forgetting the second edit impossible rather than making it
unnecessary.

A compile-time assertion inside a `const` block is unusual enough that a reader will stop
at it, so it carries a comment explaining what it proves and why it is not a test. If that
comment is ever removed, the next person to see a build fail with "the root VERSION file
disagrees with the version in Cargo.toml" will spend a while on it.

Any crate that wants the version must depend on `salman-core`. That is free for
`salman-lang`, `salman-vm` and `salman-cli`, which depend on it anyway. A future crate
that has no other reason to depend on core now has one, or must fall back to
`env!("CARGO_PKG_VERSION")` and rely on the workspace inheritance to keep it right.

`include_str!("../../../VERSION")` reaches outside the crate directory, which couples
`salman-core` to its position in this repository. Publishing `salman-core` to crates.io as
it stands would fail, because the packaged crate would not contain the file it includes.
That is a real limitation of this design and would have to be solved — most likely by
generating the constant at package time — on the day salman publishes a library crate.

## Alternatives considered

**A build script.** `build.rs` could read `VERSION` and emit the constant, and would solve
the packaging problem above cleanly. It lost on supply chain: a build script is arbitrary
code executed on every machine that builds the project, including CI runners and any
downstream consumer, and this workspace has none. Adding the first one to move a
five-character string is a poor trade.

**cargo-release, cargo-workspaces or a similar release tool.** These are good tools and
would handle the bump correctly. They lost because they are a tool someone has to
remember to run: the guarantee lives in a person's habit rather than in the build. They
also add a dependency to the release path that has to be pinned and audited like any
other.

**Trusting review.** A reviewer notices when the two numbers differ. Sometimes true, and
it costs nothing to set up, which is why it is worth naming rather than dismissing. It
lost because it fails exactly when the project is busiest, and because a mismatch is
visually boring — `0.1.0` next to `0.1.0` and `0.0.1` next to `0.1.0` look much the same
in a diff at speed.

**Cargo's workspace inheritance alone.** Already in use, and it is what keeps the member
crates consistent with `[workspace.package]`. It lost as a complete answer because it says
nothing about the root `VERSION` file, nothing about the literals in
`[workspace.dependencies]`, and nothing about what the binary prints.

## How this is enforced

* The `const` assertion in `crates/salman-core/src/version.rs`. Not a test — a build
  failure, on every machine, before any test runs.
* `crates/salman-core/src/version.rs`,
  `version_is_read_from_the_version_file_and_matches_cargo`, plus
  `version_has_no_surrounding_whitespace` and
  `version_is_three_dotted_numeric_components`, which check the file's shape rather than
  its agreement.
* `crates/salman-cli/src/main.rs`, `cli_version_string_is_the_project_version` — clap's
  `--version` output carries the `VERSION` file value.
* `.github/workflows/version-consistency.yml`, job "VERSION agrees with Cargo metadata and
  with `salman version`", whose steps check every workspace package version, every version
  literal in the workspace manifest, and the output of the built binary.
* `crates/salman-core/src/capability.rs`, capability `core.version-truth`, which cites the
  test above as its evidence and would fail the build of `salman-core` if that test were
  deleted (see ADR-0010).
