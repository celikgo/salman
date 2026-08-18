<!-- SPDX-License-Identifier: Apache-2.0 -->
# Fuzzing salman

Rule 7: untrusted input is treated as hostile. A decoder must never panic,
allocate without bound, or read out of bounds on malformed input, and every
parser is fuzzed in CI.

These targets check more than "it did not crash". Each one asserts the lexer's
postconditions — see `src/lib.rs` for the list and why each is there — so an
input that produces a structurally broken but non-panicking `TokenStream` is a
finding rather than a pass.

## Targets

| Target | Input | What it adds |
| --- | --- | --- |
| `lex_utf8` | `&str` | The baseline: the generic dialect, over the type `lex` actually takes. |
| `lex_bytes` | `&[u8]`, decoded with `String::from_utf8_lossy` | The path a real file takes. Replacement characters are three bytes, so this hammers offset arithmetic across multi-byte characters. |
| `lex_strict_dialect` | `&str` | `Dialect::strict_iec`, which enters diagnostic arms the generic dialect never reaches. |
| `lex_differential` | `&str` | Both dialects on one input: strict must never report fewer errors than generic, and the two must tokenise identically. |

## Running one

Requires a nightly toolchain (libFuzzer needs `-Z sanitizer`) and `cargo-fuzz`:

```sh
rustup toolchain install nightly
cargo install cargo-fuzz --locked
```

Then, from this directory or from the repository root:

```sh
cargo +nightly fuzz run lex_utf8
```

That runs until you stop it, starting from the seed corpus in
`corpus/lex_utf8/` and writing everything new it finds back into the same
directory. To time-box it the way CI does:

```sh
cargo +nightly fuzz run lex_utf8 -- -max_total_time=60 -timeout=10 -rss_limit_mb=2048
```

`cargo +nightly fuzz list` prints every target. `cargo +nightly fuzz build`
compiles them all without running anything, which is the fastest way to check
that a change to `salman-lang` has not broken a target.

Anything after `--` goes to libFuzzer, not to cargo-fuzz. `-jobs=8` runs eight
workers in parallel and is worth it for a long session.

## Reproducing a crash

A failing run writes the offending input to `artifacts/<target>/`, and the CI
job uploads that directory as the `fuzz-artifacts` artefact on failure. Download
it, put it back at the same path, and re-run the target against that one file:

```sh
cargo +nightly fuzz run lex_utf8 artifacts/lex_utf8/crash-2b7c…
```

That replays exactly that input and nothing else, so the panic message and
backtrace are the ones the fuzzer saw. `RUST_BACKTRACE=1` is set for you by
cargo-fuzz.

For the typed targets (everything except `lex_bytes`), the artefact is the raw
byte string libFuzzer generated, not the `&str` the target received.
`cargo +nightly fuzz fmt` prints the decoded value:

```sh
cargo +nightly fuzz fmt lex_utf8 artifacts/lex_utf8/crash-2b7c…
```

## Minimising a case

A first artefact is usually a few hundred bytes of noise with the bug somewhere
inside. `tmin` shrinks it while keeping it failing:

```sh
cargo +nightly fuzz tmin lex_utf8 artifacts/lex_utf8/crash-2b7c…
```

It writes the smallest input it reached to `artifacts/<target>/minimized-from-…`.
Minimise before opening an issue or writing a regression test: the point is to
end up with something small enough to paste into a `#[test]` in
`crates/salman-lang/src/lexer.rs`, which is where a bug found here belongs
permanently. The fuzzer finds it once; the unit test keeps it found.

To shrink the corpus itself — same coverage, fewer and smaller files — use
`cargo +nightly fuzz cmin lex_utf8`. Worth doing before committing anything new
to `corpus/`.

## The corpus

`corpus/<target>/` holds a small seed corpus, committed, drawn from the cases
the lexer's own tests say are easy to get wrong: nested comments, `1..5`,
`%QX7.5`, `T#1d2h3m4s5ms`, `16#FF` and `INT#16#FF`, string escapes, an
unterminated comment, an unterminated string, and a pragma. They exist so a
cold run starts somewhere useful instead of rediscovering that `(*` is
interesting.

CI keeps its own accumulated corpus in the Actions cache and carries it from run
to run. Local runs write into `corpus/` directly, so check `git status` before
committing: adding a few hundred fuzzer-generated files is rarely what you
meant. Keep committed seeds small, meaningful, and named after what they are.

## Why `fuzz/` is a separate workspace

`Cargo.toml` here carries its own `[workspace]` table, and `fuzz/` is not in the
root workspace's `members`. cargo-fuzz always builds `--release`; inheriting the
root release profile would apply `lto = "thin"` and `codegen-units = 1` to every
rebuild, and a whole-program LTO link on each edit is how an edit-run loop stops
being one. The consequence is that `cargo test --workspace` and
`cargo clippy --workspace` at the repository root do not cover this directory —
run `cargo +nightly fuzz build` from here to check it.
