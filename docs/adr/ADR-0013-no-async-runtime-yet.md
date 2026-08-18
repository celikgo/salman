# ADR-0013: Blocking sockets now, and which async runtime if that changes

- **Status**: Accepted
- **Date**: 2026-08-18
- **Deciders**: the salman authors

## Context

salman's first protocol needs a client and a simulator that can talk to each other over a
real socket, so that a test can drive both ends with no hardware anywhere.

The obvious move is to reach for an asynchronous runtime. Every Rust Modbus library does:
`tokio-modbus` is built on tokio, and the shape is so common that not doing it looks like an
oversight rather than a decision.

At 0.0.1 the dependency position is worth stating precisely, because the loose version of it
("salman has no dependencies") is not true.

`salman-core`, `salman-lang`, `salman-vm`, `salman-modbus`, `salman-modbus-net`,
`salman-capture` and `salman-link` depend on **nothing outside the standard library**. That
covers the Structured Text front end, the runtime, the whole Modbus stack, and the capture
reader with its link, IP and TCP decoders — which is where bytes arrive from a network.

It does **not** cover every crate that reads a file salman did not write: `salman-plcopen`
parses XML with the `xml` crate, for the reasons in
`docs/adr/ADR-0015-xml-parser.md`. An earlier revision of this ADR claimed the property held
for every decoding crate, and once PLCopen XML arrived that stopped being true. Naming the
crates is more useful than a slogan and harder to be quietly wrong about. SHA-256, the pseudo-random generator, the diagnostic renderer,
the CRC, the pcap container and every protocol decoder are written in-crate, so
`unsafe_code = "forbid"` is provable across all of it and a fuzz finding anywhere in it is
one salman can act on.

Three crates do have third-party dependencies, and it is worth being exact about which and
where, because the loose version of this claim was wrong in an earlier revision of this ADR:

- `salman-cli` parses arguments with `clap`.
- `salman-test` reads its YAML test format with `serde` and `serde-saphyr`.
- `salman-project` reads the YAML project file with the same two.

`salman-project` **is** in the path of what this ADR is about: `salman-link` depends on it,
and `salman-link` is what drives `salman-modbus-net` at the scan boundaries. So a project run
through the link does parse a file with third-party code. The distinction that still holds,
and the one that matters, is between a file a user wrote on their own machine and bytes that
arrived from a network: no third-party parser is on the second path.

## Decision

**`crates/salman-modbus-net` uses `std::net` with a thread per connection. salman takes no
asynchronous runtime at v0.2.**

What v0.2 needs is a client that issues a request and waits for its answer, and a simulator
that answers a handful of connections in a test. Blocking sockets do that in about four
hundred lines, with `set_read_timeout` for the deadline. The pure protocol layer — every
decoder, the framer, the device model — already has no I/O in it at all, so the part that
would benefit from asynchrony is the part that is four hundred lines long.

**When that stops being true, the answer is `tokio`**, with `default-features = false` and
features `["net", "time", "rt"]`. The reasons, measured rather than assumed:

- **It is the smallest.** `cargo tree` gives tokio 5 crates on Linux and 6 on macOS and
  Windows. `smol` is 31. `async-io` with `async-net` is 24. `tokio-modbus` is 24. The
  argument that usually favours the alternatives — a smaller dependency tree — favours tokio
  here, and not narrowly.
- **It has a virtual clock.** The `test-util` feature's `tokio::time::pause()` and
  `advance()` freeze `Instant::now()` and skip to the next timer when idle, which makes
  timing behaviour — response deadlines, retry backoff, RTU's 3.5-character silence —
  testable without sleeping and identically on every machine. salman already holds that
  standard for its scan runtime, and nothing else in the async ecosystem offers it.
- **Not `#[tokio::main]`.** The `macros` feature pulls in syn, quote, proc-macro2,
  unicode-ident and tokio-macros — five crates, all in `cargo-deny`'s scope — to save one
  line. `Runtime::block_on` is that line.

What would make it necessary: a client that keeps many connections open at once, a simulator
that has to hold hundreds, or serial RTU, where the 1.5- and 3.5-character silences at 9600
baud are 1.7 ms and 4.0 ms and a thread per port with a sleep loop is the wrong instrument.

## Consequences

**A thread per connection.** Fine for a simulator in a test and for a client that talks to
one device. Not fine for a scanner talking to two hundred, which salman does not do and, per
`docs/adr/ADR-0004-network-scope.md`, will not do outside explicitly declared ranges.

**Timing is the operating system's.** `set_read_timeout` gives a deadline with the
granularity the platform gives it, which is adequate for a response timeout measured in
hundreds of milliseconds and would not be adequate for RTU inter-character timing. That is
named above as one of the conditions that changes this decision.

**The decision is cheap to revisit** precisely because the protocol layer has no I/O. Nothing
in `salman-modbus` would change; `salman-modbus-net` is the whole of what an async runtime
would touch.
