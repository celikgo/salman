---
name: releasing-salman
description: How salman's nine workflows interact, the one source of version truth and the compile-time assertion behind it, what release.yml builds and publishes and in what order, the perf budget and supply-chain gates, and the ordered list of things a maintainer must do by hand — regenerate the three generated documents, refresh the README performance table, write RELEASE_NOTES.md. Use when cutting a release, tagging a version, changing a version number, editing any file under .github/workflows/, editing perf-budget.toml or deny.toml, or when a workflow is red and you need to know what it actually gates.
---

# Releasing salman

A release is one tag push. Everything after it is automated, and almost everything before it
is not. This skill is the second half.

## The nine workflows

| File | `name:` | Runs on | Failing it means |
|---|---|---|---|
| `ci.yml` | `ci` | push to main, **PR**, dispatch | the tree does not build, does not test, is not formatted, or clippy found something |
| `determinism.yml` | `determinism` | push, **PR**, dispatch | the test suite failed on Linux, macOS or Windows — which includes the committed golden trace. Its step *named* for the trace comparison is a placeholder; see the `preserving-determinism` skill |
| `docs-links.yml` | `docs-links` | push, **PR**, dispatch | a URL in a `.md` file **or in a Rust doc comment** does not resolve |
| `interop.yml` | `interop` | push, **PR**, dispatch | salman's Modbus disagreed with pymodbus 3.15.0 in one of the two roles |
| `perf.yml` | `perf` | push, **PR**, dispatch | a measurement exceeded its ceiling in `perf-budget.toml` |
| `supply-chain.yml` | `supply-chain` | push, **PR**, dispatch, **daily 05:41 UTC** | `cargo deny` found an advisory, an unlisted licence, a banned crate or an unexpected source — or an ignore in `deny.toml` states no reason |
| `version-consistency.yml` | `version-consistency` | push, **PR**, dispatch | a version literal somewhere disagrees with `crates/salman-core/VERSION` |
| `fuzz.yml` | `fuzz` | **daily 03:27 UTC**, dispatch — *not* on PRs | a fuzz target found a crash, or the `fuzz/` directory lost its targets |
| `release.yml` | `release` | **`v*` tag**, dispatch with a tag input | the release did not ship |

`CONTRIBUTING.md` says "all eight workflows must be green" and lists `fuzz` among them.
Seven of those eight actually run on a pull request; `fuzz` runs nightly and on dispatch, so
on a PR it is green because it did not run. If you want it before merging, dispatch it.

`supply-chain.yml` states the caveat that applies to both scheduled jobs, and it is worth
knowing before reading a green Actions tab as an all-clear: *"scheduled workflows run only on
the default branch, GitHub drops scheduled runs under load, and on a public repository they
are disabled automatically after 60 days without repository activity."*

## One source of version truth

`crates/salman-core/VERSION` contains the version and nothing else. It is authoritative, and
`crates/salman-core/src/version.rs` makes that true at compile time rather than in CI:

```rust
const VERSION_FILE: &str = include_str!("../VERSION");
pub const VERSION: &str = trim_ascii_end(VERSION_FILE);

const _: () = assert!(
    str_eq(VERSION, env!("CARGO_PKG_VERSION")),
    "the VERSION file disagrees with the version in Cargo.toml"
);
```

A `const` assertion, so a mismatch is a build failure on every machine — not a job someone
might skip. `trim_ascii_end` and `str_eq` are hand-written because neither `str::trim_end`
nor `==` is `const`.

The file lives **inside the crate**, not at the repository root. That is the fix recorded in
`docs/adr/ADR-0008-one-version-truth.md`: an `include_str!` reaching above the package
directory compiles in a git checkout and fails for everyone who installs from crates.io,
which would make the guarantee hold only for the people who least need it.

`version-consistency.yml` checks what the compiler cannot see: every workspace package
version, every version literal in the workspace manifest, and the output of `salman version`.

**"One source of truth" means one file is authoritative — not that one file is the only file
you edit.** `env!("CARGO_PKG_VERSION")` resolves through `salman-core`'s
`version.workspace = true` to `[workspace.package] version` in the root `Cargo.toml`, so a
bump that touches only `VERSION` does not compile. The root manifest holds thirteen version
literals: the workspace package version, and the twelve `[workspace.dependencies]` entries
that pin each salman crate for crates.io. All of them move together.

`CONTRIBUTING.md`'s shorter phrasing — *"change it in `crates/salman-core/VERSION` only"* — is
about which file decides, and the sentence after it is the operative one: `salman-core` fails
to compile if Cargo disagrees. That failure is the feature. It means you cannot ship a
half-bumped tree, only fail to build one.

## What `release.yml` does

Trigger: a pushed tag matching `v*`, or `workflow_dispatch` with an existing tag as input.

**1. `check-version` — the gate.** Reads `crates/salman-core/VERSION`, compares it to the tag,
and fails with `tag $tag does not match VERSION ($version); expected v$version`. Nothing else
starts until it passes.

**2. `build` — four target triples**, `fail-fast: false`:

| Target | Runner | Note |
|---|---|---|
| `x86_64-unknown-linux-gnu` | ubuntu-latest | oldest glibc GitHub offers, so the binary runs on older distributions |
| `aarch64-apple-darwin` | macos-latest | native |
| `x86_64-apple-darwin` | macos-latest | **cross-compiled**; smoke-tested through Rosetta, and if that ever stops working the leg uploads the binary with a warning that it is *unsmoked* rather than silently skipping the check |
| `x86_64-pc-windows-msvc` | windows-latest | native |

Every leg runs the binary and asserts it prints exactly `salman <version>`. Then it packages
`salman` plus `LICENSE`, `LEGAL.md` and `README.md` into
`salman-<tag>-<target>.tar.gz` (or `.zip` on Windows) with a `.sha256` beside it. `LEGAL.md`
travels in the archive deliberately: it is where the non-certification statement lives, and
that statement should reach the downloader rather than staying on a web page they may never
open.

**3. `publish-crates` — crates.io, last.** `needs: [check-version, build]`, because it is the
one step that cannot be undone: a GitHub release can be re-cut and a crates.io version cannot.
It checks `CARGO_REGISTRY_TOKEN` is present with a message naming where to set it, then runs
a single `cargo publish --workspace --locked`. **No hand-maintained publish order exists**, and
that is deliberate: `cargo publish --workspace` computes the topological order from the
dependency graph, so nothing rots the first time a crate gains a dependency.

**4. `publish` — the GitHub release.** Collects every archive and concatenates the checksums
into one sorted `SHA256SUMS`. Creates the release as a **draft**, so a person reads the notes
before anyone else does, with `RELEASE_NOTES.md` as the body. If the tag already has a
release it uploads to it with `--clobber` instead of failing — which is how a tag cut before
crates.io publishing existed gets pushed to the registry without leaving a red job beside a
green one.

`publish-crates` and `publish` both depend only on `check-version` and `build`, so they run in
parallel. There is no provenance attestation and no signing; the SHA-256 sums are the
integrity story.

## The gates worth understanding before you trip them

### `perf.yml` and `perf-budget.toml`

Four measurements per platform, with per-OS thresholds because the three runners are not three
copies of one machine — different instruction set, different malloc, different page size
(macOS aarch64 is 16 KiB), a PE binary with its own section alignment on Windows.

| Key | Budget | What it measures |
|---|---|---|
| `cold_start_ms` | 2000 | `salman version`, process start to exit |
| `peak_rss_bytes` | 350 000 000 | peak resident set of that same invocation |
| `binary_size_bytes` | 120 000 000 | the release `salman` binary on disk |
| `test_suite_s` | 60 | `cargo test --workspace`, excluding the build |

Two honest caveats live in that file rather than in a footnote, and both should survive any
edit:

- **`binary_size_bytes` is an installer budget being spent on a binary.** There is no
  installer, so the job weighs the CLI against the same 120 MB number. The comment says so
  explicitly *"so that nobody later reads a passing run as evidence that the installer fits"*.
  When an installer lands it gets its own key and this one gets a number that means something.
- **`peak_rss_bytes` is deliberately absent on Windows**, and `perf.yml` measures none there.
  The runner image has no `/usr/bin/time`, and every PowerShell alternative for a
  few-millisecond process is racy. A number obtained by racing the process would be a number
  nobody could act on.

The thresholds are **ceilings, not targets** — set to catch a change that makes salman an
order of magnitude worse, not fifteen percent worse, because a gate that fails on
shared-tenant runner noise gets ignored, then disabled, then deleted. **Do not weaken them.**
Raising one is a reviewed change to `perf-budget.toml`, which is the point: the budget can
only move in a commit that says it is moving.

Units are in the key names on purpose: GNU `/usr/bin/time -f %M` reports kilobytes and BSD
`/usr/bin/time -l` reports bytes, and reading one as the other is a 1024× error.

### `supply-chain.yml` and `deny.toml`

Two jobs. The first, *"deny.toml is reviewable"*, fails if any ignore or exception states no
reason. The second runs `cargo-deny` as four separate matrix legs — `advisories`, `licenses`,
`bans`, `sources` — split so the log says which policy failed without anyone reading it.

The licence list is an **allowlist**: `Apache-2.0`, `MIT`, `Unicode-3.0`, `BSD-2-Clause`,
`BSD-3-Clause`, `ISC`, `Zlib`, `NCSA`. A licence salman has not considered fails the build
rather than entering quietly. `NCSA` is listed forward-lookingly for `libfuzzer-sys`, which
is why the `licenses` leg passes `--allow license-not-encountered` — an allowlist entry with
nothing to match is not a supply-chain problem, and an unlisted licence still is.

Adding a dependency means adding a row to `LEGAL.md` §8 as well.

## The maintainer's manual checklist

In order. **Steps 2 and 4 are the ones nothing checks for you at all.** Step 1 is caught
after the fact by the three drift tests, and step 3 is checked only for the file's
*existence*, never its contents.

1. **Regenerate the three generated documents**, and commit whatever moved.

   ```bash
   cargo build --release
   ./target/release/salman status --markdown > docs/STATUS.md
   SALMAN_UPDATE_GOLDEN=1 cargo test -p salman-plcopen --test compat
   # docs/IEC_CITATIONS.md has no writer — see the citing-the-standard skill
   ```

   `docs/IEC_CITATIONS.md` only changes if `clause.rs` changed, and its drift test will tell
   you. `docs/STATUS.md` changes whenever the capability registry does, which on this project
   is most releases.

2. **Refresh the README performance tables.** There are two, and they currently name two
   *different* commits — `624e176` beside "What it costs to run" and `d85c241` beside "The
   performance budget". Nothing updates either automatically. Open the `perf` run for the
   commit you are tagging, copy its four measurements per platform, and update both tables and
   both commit hashes. Keep the precision honest:
   the file already explains that three runs of one commit measured macOS cold start at
   0.93 ms, 1.21 ms and 4.26 ms, and that publishing two decimal places would imply a
   precision that is not there.

3. **Write `RELEASE_NOTES.md`.** `release.yml` fails with *"RELEASE_NOTES.md is missing; the
   notes are written by hand from docs/STATUS.md"* if it is absent — but it does not check
   that the contents describe *this* version. The existing structure, which is worth keeping:

   ```
   # salman v0.1.0
   ## Install
   ## What works
   ## What is not implemented
   ## What salman does not claim
   ## Measured cost
   ## Since the tree was first pushed
   ## Supported versions
   ```

   Three of those seven sections are about limits. That ratio is the house style, and it is
   the section a reader of a pre-alpha most needs.

4. **Check the honesty of every claim that moved.** `README.md`, `docs/ROADMAP.md`,
   `docs/CONFORMANCE.md` and `RELEASE_NOTES.md` are all hand-written and all make claims. In
   particular: does the README still describe what `cargo install salman-cli` can actually do
   *today*? Before the first successful `publish-crates` run there is nothing on crates.io,
   and the README says so rather than promising a command that would fail in front of exactly
   the reader the quick start exists for.

5. **Bump the version**, in `crates/salman-core/VERSION` *and* in the root `Cargo.toml` —
   `[workspace.package] version` plus the twelve `[workspace.dependencies]` literals. The
   `const` assertion fails the build if you miss the manifest; `version-consistency.yml`
   catches the rest. `cargo build --workspace` before you commit is the fastest check.

6. **Push the tag.**

   ```bash
   git tag v0.2.0
   git push origin v0.2.0
   ```

7. **Watch `release.yml`.** `check-version` fails fast if the tag and the file disagree.

8. **Read the draft release, then publish it.** It is a draft on purpose.

## Current state of the release path

Accurate at 0.1.0, and worth re-checking rather than assuming:

- **Nothing is on crates.io yet.** The workspace is packaged — every crate carries the
  metadata crates.io asks for and its own README, and `cargo package --workspace` verifies all
  thirteen — and `publish-crates` is wired, but until a tag runs it successfully
  `cargo install salman-cli` will not find salman. The releases page is the way to get a
  binary without a Rust toolchain.
- **There is no installer**, which is why `binary_size_bytes` is budgeted against the wrong
  thing on purpose and says so.
- **There is no signing and no provenance attestation.** Signed installers are a v0.3 item in
  `docs/ROADMAP.md`.
- **The `x86_64-apple-darwin` leg is cross-compiled** and depends on Rosetta being present on
  the runner for its smoke test.

## If you are editing a workflow

Eight of the nine carry a long comment block explaining what they gate and, more usefully,
what they do *not*; `ci.yml` is the exception, and its two-line header explains only why it
exists at all. `determinism.yml` explains why its placeholder step exists;
`fuzz.yml` explains why it fails when the `fuzz/` directory is empty and why its `--target` is
pinned to `x86_64-unknown-linux-gnu`; `docs-links.yml` explains its two exclusions one at a
time; `perf-budget.toml` explains why its numbers are generous. Those comments are the
argument. If your change makes one of them false, the comment is part of the change.

Do not weaken the perf budget, the determinism gate, or any fuzz target.
