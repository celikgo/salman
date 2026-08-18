# ADR-0005: Determinism strategy

- **Status**: Accepted
- **Date**: 2026-08-18
- **Deciders**: the salman authors

## Context

A golden-trace test is worth having only if the trace is stable. If a simulation produces a
slightly different trace on the reviewer's laptop than on the build agent, the test tells
you nothing and everyone learns to ignore it. Rule 6 in `README.md` states the promise:
same project, same inputs, same seed, identical trace, bit for bit.

Nothing about that promise is free. A general-purpose language on three operating systems
and two processor architectures offers several quiet ways to break it, and every one of
them produces a green test suite on the machine that introduced the defect:

- The bit pattern a processor produces for a NaN is not portable. On aarch64, `0.0 / 0.0`
  evaluated at run time yields `0x7ff8_0000_0000_0000`; on x86-64 the default quiet NaN is
  the negative-signed indefinite, `0xfff8_0000_0000_0000`. That was checked by running it,
  not reasoned about, and a single division would break a byte comparison on its own.
- Rust's transcendental functions delegate to whatever library the target links. The
  documentation for [`f64::sin`](https://doc.rust-lang.org/std/primitive.f64.html) states
  that the precision "varies by platform, Rust version, and can even differ within the same
  execution from one invocation to the next".
- `HashMap` and `HashSet` default to
  [`RandomState`](https://doc.rust-lang.org/std/collections/hash_map/struct.RandomState.html),
  initialised with random keys, so iteration order differs between two runs of the same
  binary on one machine.
- Rendered text, wall-clock reads and thread scheduling each carry their own hazard.

## Decision

The same project, the same inputs and the same seed produce a byte-identical trace on Linux
x86-64, macOS aarch64 and Windows x86-64. Seven mechanisms hold that up, and each is in the
tree now.

1. **NaN is canonicalised on entry to a `Value`.** `canonical_f32` and `canonical_f64` in
   `crates/salman-core/src/value.rs` collapse every NaN to one quiet NaN with a zero payload
   — `0x7fc0_0000` and `0x7ff8_0000_0000_0000` — and `Value::real`, `Value::lreal` and
   `write_canonical_bytes` all apply them. Negative zero is deliberately preserved: unlike
   NaN it is portable, and `1.0 / -0.0` is `-inf`.
2. **Only the exactly-specified floating-point operations are used.** Rust guarantees IEEE
   754 semantics for `+ - * / %` and `sqrt`, and does not contract `a*b+c` into an FMA,
   which C compilers on Arm do by default. Transcendental functions are banned from the
   evaluation path by `clippy.toml`, which lists `sin`, `cos`, `tan`, the inverse
   trigonometric functions, `exp`, `ln`, `log`, `powf`, `powi`, `cbrt` and `hypot` for both
   `f32` and `f64`. When salman implements them it will use the pinned pure-Rust
   [`libm`](https://docs.rs/libm/latest/libm/) crate. Stated plainly: `libm` documents no
   bit-exactness guarantee, so the defensible claim is one pinned implementation, so results
   do not vary with the host C library — not deterministic transcendental functions.
3. **`std::collections::HashMap` and `HashSet` are banned by `clippy.toml`** from anything
   that can reach a trace, a diagnostic or a generated file. `BTreeMap` and `BTreeSet` are
   the substitutes; `crates/salman-lang/src/types.rs` and `crates/salman-vm/src/memory.rs`
   use them. A site that genuinely wants a hash map for lookup only passes an explicit
   deterministic hasher and allows the lint with a comment saying why order cannot escape.
4. **The trace fingerprint is SHA-256 over a canonical binary encoding**, not over rendered
   text. `Value::write_canonical_bytes` writes a type tag then little-endian bytes, so
   `Value::Int(1)` and `Value::Dint(1)` cannot hash the same. Rust's float formatting is
   pure Rust and platform-identical, but carries no cross-version stability promise and has
   changed before, so hashing bit patterns takes formatting out of the argument entirely.
   The hash is written out in `crates/salman-core/src/hash.rs` rather than taken from a
   dependency, to keep runtime CPU-feature dispatch out of the thing that decides whether
   platforms agree.
5. **The virtual clock.** `crates/salman-vm/src/clock.rs` advances only when the scheduler
   says so, and its epoch is configured rather than taken from the host. Real-time mode
   exists for hardware-in-the-loop work, makes no determinism promise, and reports its
   measured jitter instead of pretending there is none.
6. **Single-threaded evaluation.** Floating-point addition is not associative, so any
   parallel reduction reassociates according to thread scheduling and cannot produce a
   reproducible answer. Ties between tasks released at the same instant are broken by
   declaration order in `crates/salman-vm/src/task.rs`, so the answer never depends on how
   a collection happened to iterate.
7. **The build is pinned and the bytes are left alone.** `rust-toolchain.toml` pins the
   compiler exactly; `.gitattributes` sets `eol=lf` for tracked text and `-text` for golden
   artefacts, so git cannot turn the determinism gate into a line-ending test on Windows.

## Consequences

The honest scope statement, which is the only claim salman makes: bit-identical traces are
gated for x86_64-unknown-linux-gnu, aarch64-apple-darwin and x86_64-pc-windows-msvc on the
pinned toolchain, for programs whose floating-point operations stay within the
exactly-specified set, with NaN canonicalised at the value boundary. Nothing is claimed for
32-bit targets, for parallel evaluation, or across toolchain upgrades.

Cross-OS artefact determinism is currently an untested premise — the determinism workflow
exists to discover whether it holds, and salman must not claim it until that job has been
green on all three operating systems for a meaningful period. Today
`.github/workflows/determinism.yml` runs the test suite on three platforms and then prints
a warning saying that it compared no trace. The blocker is no longer the runtime: `salman
run` exists and writes a trace, and `crates/salman-vm/src/task.rs`,
`the_same_configuration_run_twice_produces_the_same_trace_fingerprint`, shows two runs on
one machine agree. What is missing is the cross-platform half — per-OS artefact upload and
a fan-in job comparing the three byte for byte — and until that is written the green tick on
that workflow means "the tests passed everywhere", not "the traces matched".

The costs are real. Bumping the Rust toolchain invalidates the premise until the gate has
run again, so a compiler upgrade is a reviewed change with a determinism argument attached
rather than a routine bump. `SIN` and `LOG` cannot be implemented until `libm` is added,
and when they are, their results will be reproducible without being provably correctly
rounded. `BTreeMap` is slower than `HashMap` on the lookup-heavy paths in the type checker,
and that cost is paid on every compile. Single-threaded evaluation forecloses parallel
simulation of independent tasks, a real performance ceiling on large projects. Writing
SHA-256 in-crate means maintaining a cryptographic-shaped primitive that is explicitly not
a security primitive. And a scan is atomic, so no amount of determinism lets salman
reproduce a race that depends on being interrupted mid-scan: determinism is not fidelity.

## Alternatives considered

**Compare traces as rendered text.** Simpler, and it makes the diff in a pull request the
same artefact the gate compares. It lost because it puts Rust's float formatting inside the
determinism promise: the formatting is platform-identical today but carries no cross-version
guarantee and has changed before, so a compiler upgrade could break the gate for a reason
unrelated to the simulation. salman renders text for humans and hashes bytes for the gate.

**Use a tolerance instead of bit equality.** Compare floats to within an epsilon and accept
platform variation. This is what most simulation tools do and it is defensible for physics.
It lost because a bit-exact comparison detects any change, including the ones a tolerance
was chosen to hide, and choosing the tolerance becomes an argument nobody can settle.

**Fixed-point or rational arithmetic throughout.** Would remove the floating-point problem
at the root. Rejected because IEC 61131-3:2013 (Edition 3.0) references IEEE 754 normatively
for `REAL` and `LREAL`, and a runtime computing something else would be deterministically
wrong. That edition was withdrawn on 2025-05-22 and superseded by IEC 61131-3:2025
(Edition 4.0); salman targets Edition 3.0 because it is the edition our public sources let
us verify.

**Rely on review and a single reference platform.** Run the gate on Linux only and trust
that the others agree. Rejected because the failure modes listed in the Context are exactly
the ones that pass on one platform. A single-platform gate would have caught none of them.

## How this is enforced

- `.github/workflows/determinism.yml` runs `cargo test --workspace --all-features` on
  `ubuntu-latest`, `macos-latest` and `windows-latest` with `fail-fast: false`, and prints a
  warning on every run stating that the trace comparison itself is not yet implemented. That
  placeholder is now unblocked — `salman run` produces a trace — and what replaces it is a
  per-OS artefact upload and a job that compares the three byte for byte. That work has not
  been done, so the warning is still what the job prints.
- `clippy.toml` bans the transcendental functions via `disallowed-methods` and `HashMap` /
  `HashSet` via `disallowed-types`, each with the reason next to the rule.
  `.github/workflows/ci.yml` runs clippy with `-D warnings`, so a violation fails the build.
- `crates/salman-core/src/value.rs`:
  `nan_is_canonicalised_so_traces_cannot_differ_between_architectures`,
  `negative_zero_is_preserved_because_it_is_portable_and_meaningful`,
  `canonical_bytes_are_tagged_so_two_types_holding_one_cannot_collide`;
  `crates/salman-vm/src/trace.rs`: `a_trace_contains_nothing_ambient`,
  `a_rendered_trace_contains_no_carriage_returns_on_any_platform`;
  `crates/salman-core/src/hash.rs`:
  `the_published_fips_180_4_vectors_hash_to_their_published_digests`.
- `rust-toolchain.toml` pins the compiler; `.gitattributes` fixes line endings and exempts
  golden artefacts from normalisation. Neither is checked by a test; both would be noticed
  only by the gate failing.
