# ADR-0001: Rust as the implementation language

- **Status**: Accepted
- **Date**: 2026-08-18
- **Deciders**: the salman authors

## Context

salman exists to read things it did not write. Project archives exported by vendor
tooling, source files from a plant that has been running since 2004, and — once the
protocol layer arrives — frames arriving off an industrial network. Every one of those
is an untrusted byte string, whatever its provenance claims.

That places the whole project on the wrong side of a well-known line. A heap overflow in
a protocol decoder that is listening on a plant network is not a crash; it is a remote
code execution vector on a machine that has a route to controllers. The same defect in a
project-file importer is a code execution vector reached by sending somebody a file. The
history of industrial tooling contains a great many of both.

Memory safety is therefore a security property of this project, not a matter of taste or
of developer comfort. It is not something a code review can be relied upon to supply,
because the reviews that failed to supply it elsewhere were conducted by competent people
who were paying attention.

Three further forces were in play. salman must ship as a single artefact an engineer can
copy onto a locked-down workstation, which rules out a language runtime that has to be
installed first. Its scan loop must run without a garbage collector deciding to pause in
the middle of it. And the protocol layer will eventually need to load C plugins, so the
implementation language must have a first-class, stable C ABI story rather than a
foreign-function bridge.

## Decision

salman is written in Rust, on the stable channel, using the 2024 edition, with the
compiler version pinned exactly in `rust-toolchain.toml`. The workspace forbids `unsafe`
outright rather than discouraging it.

Specifics:

- `rust-toolchain.toml` pins `channel = "1.94.1"` and requests `rustfmt` and `clippy`.
  The pin is exact, not a floor: the determinism gate compares artefacts produced by
  different machines, and that comparison means nothing if the machines run different
  compilers. See [ADR-0005](ADR-0005-determinism.md).
- `Cargo.toml` sets `edition = "2024"` for the workspace. See the
  [Rust edition guide](https://doc.rust-lang.org/edition-guide/rust-2024/index.html).
- `[workspace.lints.rust]` sets `unsafe_code = "forbid"`, and every crate opts in with
  `[lints] workspace = true`. `forbid` is chosen over `deny` deliberately: `deny` can be
  overridden by an `#[allow]` further down the tree, and `forbid` cannot. The lint also
  covers `no_mangle`, `export_name` and `link_section`, per the
  [rustc lint listing](https://doc.rust-lang.org/rustc/lints/listing/allowed-by-default.html).
- Panicking constructs are denied in library code by the same block: `unwrap_used`,
  `expect_used`, `panic`, `todo` and `unimplemented`. A decoder that aborts the process
  on malformed input has traded a memory-safety bug for an availability bug.
- Graph-shaped data is expressed with arena indices and typed identifier newtypes rather
  than with references. `TypeId` in `crates/salman-lang/src/types.rs` and `SlotId` in
  `crates/salman-vm/src/memory.rs` are the pattern.

## Consequences

Compile times are worse than C's and far worse than Go's. This is felt most in the
edit-test loop of the language front end, which is the largest crate. The `dev` profile
is set to `opt-level = 1` to keep the test suite inside its budget, which is itself a
compromise: it makes debug builds slower to produce.

The pool of automation engineers who write Rust is much smaller than the pool who write C
or C++. A contributor from the domain who knows exactly what a `TON` should do may still
be blocked by the borrow checker. That is a real cost to a project whose value depends on
domain knowledge reaching the code.

The borrow checker makes some ordinary data structures awkward. An abstract syntax tree
with parent pointers is the obvious example, and salman does not have one: it uses arena
indices and node identifiers instead. That is a workable pattern but it is a workaround,
it costs a layer of indirection at every access, and it moves a class of error from
compile time to a bounds check.

Binding to the existing C fieldbus stacks will require FFI work that a C++ project would
get for free. Every one of those bindings will need an `unsafe` boundary, and under
`forbid` that boundary cannot live in this workspace — it will need its own crate and its
own ADR. That is the intended friction, and it is still friction.

Forbidding `unsafe` also forecloses some optimisations outright. If a hot path ever needs
one, the answer has to be a better algorithm or a separate reviewed crate, not an
`#[allow]`.

Finally, `unsafe_code = "forbid"` says nothing about the dependency graph. Crates
underneath salman may contain any amount of `unsafe`. That gap is why `deny.toml` and
`.github/workflows/supply-chain.yml` exist, and it is a different problem with a
different gate.

## Alternatives considered

**C++** is the incumbent in this domain, and that is a serious argument for it: the
existing fieldbus stacks, the vendor SDKs and the available engineers are all there. It
lost on the one point the project cannot compromise on. The whole reason for choosing an
implementation language here was memory safety in a byte parser, and C++ does not provide
it. Modern C++ with sanitisers, static analysis and discipline reduces the defect rate; it
does not change the class of defect that is possible.

**Go** has an excellent story for single static binaries and cross-compilation, and would
have been the fastest of the candidates to write. It lost on two counts. Garbage
collection pauses in a scan loop are not acceptable in a tool whose central promise is
reproducible timing, and cgo makes the C plugin ABI substantially more awkward than Rust's
`extern "C"`.

**Zig** is a good fit on the technical merits — comptime, C interoperability, no hidden
allocation — and its story for a single small binary is arguably better than Rust's. It
lost on age. salman is a bet on a decade, and Zig has not yet reached 1.0; its standard
library and language surface are still moving. This is a judgement about timing, not about
quality, and it may read differently in five years.

**C# / .NET** is where the vendor tooling ecosystem actually lives, which would have made
integration with existing engineering suites much easier. It lost because it is a poor fit
for what salman wants to be: a small cross-platform binary with no runtime to install,
running equally on a Linux build agent and an engineer's macOS laptop. Choosing it would
also have pulled the project toward the Windows-only assumptions that
[ADR-0003](ADR-0003-plcopen-xml-canonical.md) rejects for other reasons.

## How this is enforced

- `Cargo.toml`, `[workspace.lints.rust]`: `unsafe_code = "forbid"`. Every crate's
  `Cargo.toml` carries `[lints] workspace = true`; a crate that omits it opts out of the
  entire lint block, so that line is load-bearing.
- `rust-toolchain.toml` pins the compiler to an exact version. Changing it is a reviewed
  change, and the file says so.
- `.github/workflows/ci.yml`: the `clippy` job runs
  `cargo clippy --workspace --all-targets --all-features -- -D warnings`, which promotes
  the rustc lints above to errors as well as clippy's own. The `test` job builds and tests
  on `ubuntu-latest`, `macos-latest` and `windows-latest` with `fail-fast: false`.
- `.github/workflows/supply-chain.yml` with `deny.toml` gates what enters the dependency
  graph, on push and on a daily schedule.
- `.github/workflows/fuzz.yml` with the six targets in `fuzz/fuzz_targets/` fuzzes the
  lexer, the parser, and the three passes together through semantic analysis. Memory safety
  from the language and fuzzing of the front end are complementary: the first rules out a
  class of exploit, the second finds the panics and hangs that remain.
