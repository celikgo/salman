---
name: adding-a-fieldbus-protocol
description: The architectural seam a second fieldbus protocol sits on — the sans-I/O codec crate versus the socket crate, what is protocol-agnostic in salman-capture and what is Modbus-specific in salman-analyse, where salman-project and salman-link are still hard-wired to Modbus, the posture gate every write must pass, and CONTRIBUTING's eight-point bar. Written as a design review before a second protocol exists. Use when adding OPC UA, CANopen, EtherCAT, EtherNet/IP, S7comm or any protocol; when changing salman-modbus or salman-modbus-net; or when deciding where protocol-specific code belongs.
---

# Adding a fieldbus protocol

Modbus is the only protocol in the tree. This document exists so the second one does not have
to be reverse-engineered from the first, and so that the places where the first one leaked
are visible before they get copied.

**There is no plugin trait.** No `trait Protocol`, no registry, no dynamic dispatch. What
exists is a shape that Modbus follows, and four crates that name Modbus in their
dependencies. Adding a protocol means following the shape and generalising two of those
crates. Saying so plainly is more useful than pretending an abstraction exists.

`CONTRIBUTING.md` holds the eight-point bar a new protocol must clear. Read it; this skill
is the architecture behind it.

## The split that works: sans-I/O

Two crates, and the boundary is the good part of the existing design.

**`salman-modbus`** — 4 771 lines. Its module doc states the contract:

> This crate is **pure**: it opens no socket, reads no file and starts no thread. Bytes go in
> and typed frames come out, which is what makes the same decoder usable on a live socket and
> on a capture file. A decoder that could only be exercised against real equipment could not
> be tested at all.

Its only dependency is `salman-core`. No third-party crates. Public surface:

```rust
pub use crc::Crc16;
pub use device::{BitTable, Device, Table, WordTable};
pub use function::{ExceptionCode, FunctionCode};
pub use pdu::{Bits, DecodeError, EncodeError, Pdu, Request, Response, Words};
pub use rtu::{RtuAdu, RtuError};
pub use tcp::{Framer, MbapHeader, TcpAdu};
```

Note the shape: a **PDU layer** (protocol semantics, transport-free), a **framing layer** per
transport (`tcp::Framer` and `MbapHeader`, `rtu::RtuAdu` and `Crc16`), a **function code**
enum with its exception codes, a **device data model** (`Device`, `Table`, `BitTable`,
`WordTable`) that a simulator serves and a client addresses, and a **`limits`** module holding
every constant the specification fixes.

**`salman-modbus-net`** — 1 838 lines, `client.rs` and `server.rs`. Depends on `salman-core`
and `salman-modbus`, nothing else. Blocking sockets, no async runtime — see
`docs/adr/ADR-0013-no-async-runtime-yet.md`. This is the only crate in the workspace that
reaches a network, and the only place `Instant::now` appears in library code (socket
deadlines).

**Do the same.** `salman-<protocol>` pure, `salman-<protocol>-net` for the socket. The reason
is not tidiness: it is that the pure crate is the one you can fuzz, the one you can run
against a capture file, and the one that can be `unsafe_code = "forbid"` provably from the
first byte to the decoded frame.

## What is already protocol-agnostic

**`salman-capture`** (3 823 lines) depends only on `salman-core`. pcap read and write,
Ethernet, VLAN, IPv4, IPv6, TCP, and TCP reassembly. `salman-modbus` is a *dev*-dependency
only, for its tests. Public surface: `Reader`, `Writer`, `Record`, `LinkType`, `Resolution`,
`TimestampScale`, `CaptureError`, and `Reassembler`, `Stream`, `Delivery`, `Note`.

A second TCP-based protocol needs **nothing** here. It gets reassembled byte streams for
free. A protocol on a different link layer, or on serial, is a genuine extension of this
crate rather than a use of it — and that is a real cost to state in your ADR.

**`salman-findings`** (1 297 lines, `salman-core` only) is the vocabulary of claims. A
`Finding` is built through a constructor that names its own epistemic status — `fail`,
`pass`, `open`, `cannot_determine`, `not_applicable`, `informational` — and carries a `Kind`,
`Severity`, `Group`, `Confidence`, `Justification`, an optional `NextCheck` and a `Dedup`
policy, plus `Evidence` (`Artifact`, `Observed`, `TransactionRef`). A protocol adds findings;
it does not change this crate.

## Where Modbus has leaked, and what that costs you

Five crates name `salman-modbus` in `[dependencies]`. One of them is `salman-modbus-net`,
which is the point. Of the other four, two are fine and two are the work.

| Crate | Why it depends on Modbus | Verdict |
|---|---|---|
| `salman-modbus-net` | it is the transport for it | By design |
| `salman-analyse` | `src/modbus.rs` is the protocol-specific analyser | **Fine** — add a sibling module, and see the `timeline.rs` note below |
| `salman-link` | `link.rs` calls `salman_modbus_net::client::Client` directly | **Needs generalising** |
| `salman-project` | `spec.rs` and `map.rs` are typed on `salman_modbus::device::Table` and `salman_modbus::limits` | **Needs generalising** |
| `salman-cli` | the `capture` subcommand's `--modbus-port` default | Cosmetic |

### `salman-analyse` — copy the pattern

`crates/salman-analyse/src/lib.rs` states the layering that makes this crate worth imitating:

> The layers below this one produce **facts**: these bytes were at this offset, this stream
> carried these bytes, this frame decoded to this request. This layer produces **claims**, and
> every claim points back at the facts that support it.

`modbus::analyse_capture(name, bytes, options) -> Result<Analysis, CaptureError>` is the
whole Modbus-specific path: open a `Reader`, take the `LinkType` and the `TimestampScale`, run
a `Reassembler`, run one `Framer` per direction, decode, and emit findings. Your protocol gets
`src/<protocol>.rs` beside it with the same signature shape, and `lib.rs` gains a `pub mod`.

`timeline.rs` — which merges a capture and a scan trace onto one axis and labels every finding
with the scan it fell inside — is **almost** protocol-agnostic, and the gap is a one-line one
worth fixing while you are there. Its body uses nothing but `analysis.findings`, but
`Timeline::merge(trace, analysis, alignment)` is typed on `crate::modbus::Analysis`, so a
second protocol's analysis cannot be passed to it as written. Generalise the parameter rather
than copying the module.

The restraint in that module doc is also part of the pattern, and worth reading before you
write forty findings: the most prominent ICS tooling from a national cyber agency stops at
structured decoding with no anomaly detection at all, and Wireshark's mature Modbus dissectors
register four expert items between them. *"A hundred low-precision findings is how a
diagnostic tool loses the reader."*

### `salman-project` — the schema is Modbus-shaped

This is the sharpest edge, and the reason to write this document before the second protocol
rather than after.

`spec.rs` already has the right *idea*:

```rust
pub enum Protocol {
    #[serde(rename = "modbus-tcp")]
    ModbusTcp,
}
```

One variant, ready for more. But the mapping beneath it is not:

```rust
enum TableName {
    #[serde(rename = "discrete-inputs")]  DiscreteInputs,
    #[serde(rename = "coils")]            Coils,
    #[serde(rename = "input-registers")]  InputRegisters,
    #[serde(rename = "holding-registers")] HoldingRegisters,
}

struct MappingSpec { table: TableName, from: u16, count: u16, to: String }
```

`table:` is the four Modbus tables, hard-wired into the project file's schema, and `from`/
`count` are `u16` because a Modbus address is 16 bits. `map.rs` imports
`salman_modbus::device::Table` and `salman_modbus::limits` and validates against them: an
image address of a size no Modbus table can fill, a write to a table Modbus has no function
to write, a mapping larger than one Modbus request may carry.

A second protocol therefore forces a decision in the **file format**, which is a
compatibility surface. Decide it in the ADR, before code:

- Does `table:` become protocol-dependent, with `Protocol` selecting which enum is parsed?
- Or does each protocol get its own mapping-spec shape under its own key?
- What replaces `from: u16` for a protocol whose address is not 16 bits — an OPC UA node id,
  a CANopen index/sub-index pair, an EtherCAT PDO offset?
- The per-mapping validation errors in `map.rs` are Modbus sentences. Which of those checks
  is really about *this* protocol, and which is about the process image (which is
  protocol-independent and stays)?

### `salman-link` — the scan-boundary contract, and the hard rule

`salman-link` (1 011 lines) runs a project's IO mappings at the scan boundaries:
`Link::poll_inputs(&mut self, memory: &mut Memory)` before the scan latches, and
`Link::publish_outputs(&mut self, memory: &Memory)` after it publishes — both `&mut self`,
because the write goes through the owned `Client`. Both are typed on Modbus today.

`Link::new` takes an explicit `Peer` — `Simulated` or `Live` — and **refuses at construction
to hold an output mapping against a live peer**, then refuses again at the call that would
perform the write. The error is `LinkError::WouldDriveALiveDevice`.

`docs/adr/ADR-0014-salman-does-not-drive-a-plant.md` is the decision, and it applies to your
protocol without discussion:

> **Output mappings run against a simulated device only. Against live equipment, salman reads
> and does not write.**

`Peer` is a value the caller supplies because there is nothing in a socket that says whether
the thing on the other end is real. salman cannot detect this and does not pretend to. If your
protocol makes you want to relax this, the ADR names the price: a new ADR superseding
ADR-0014, answers to the watchdog and failsafe questions, and *"a reason better than 'a user
asked'"*.

## The posture gate

`crates/salman-core/src/posture.rs`. Every write goes through it; reads need no permission,
which is what read-only by default means.

- `Posture` — the operating mode. `PostureState::permits(effect, now_ms) -> Permit`.
- `Effect` — what an operation would do. Some variants are **categorically refused at every
  posture**: `Effect::is_categorically_refused` is not a configuration lookup. Firmware
  operations, credential guessing and denial of service are refused in code and are not
  settings.
- `UserConfirmation` — the load-bearing type:

  ```rust
  pub struct UserConfirmation {
      _private: (),
  }
  ```

  **No public constructor.** The only way to obtain one is
  `ConfirmationRequest::ask(&self, prompt: &mut dyn ConfirmationPrompt) -> Option<UserConfirmation>`,
  and `ConfirmationPrompt` is *"Implemented by the desktop app and by an interactive CLI. An
  agent must be given one; it cannot be one."*

`salman_modbus_net::Client::write` takes a `UserConfirmation` **by value**, so one
confirmation authorises exactly one write and cannot be kept. Your client's write does the
same. Passing by reference, storing one, or deriving `Clone` on it would convert a per-call
confirmation into a session-wide licence — which is precisely the failure ADR-0014 describes.

The posture model was written before anything could reach a network, so that the first write
path could not be written without going through it. Keep that true.

## The obligations, and what they actually cost

`CONTRIBUTING.md`'s eight-point bar, with the real price tag from Modbus.

1. **An ADR first**, covering the addressing model, the endianness decisions, and what the
   specification leaves ambiguous. `docs/adr/ADR-0012-modbus-addressing.md` is the model: on
   the wire salman always uses the PDU address and applies no transformation of any kind, and
   it shows the user that same address everywhere — *"The mapping between what a user types
   and what goes on the wire is the identity. There is no offset to get wrong, because salman
   does not apply one."* Any column carrying an address is named so its convention is visible
   (`pdu_addr`, never a bare `address`). Every protocol faces some version of this question.
   Ambiguity is normal; deciding it silently is not.

2. **No vendored third-party stack.** salman writes its own decoders. It buys a fuzz target
   salman owns, no `unsafe` between a socket and a decoded frame, and a simple licence
   position. `LEGAL.md` §8 permits a GPL/LGPL implementation as an external differential-testing
   oracle run as a **subprocess**, never linked.

3. **Framing and decoding tested against real captures**, not only against salman's own
   encoder. A decoder checked only against its matching encoder tests that the pair agree, not
   that either is right. `examples/capture/conveyor.pcap` is the existing worked example.

4. **An interop job** against an independent implementation, in both roles.
   `.github/workflows/interop.yml` pins `pymodbus==3.15.0` and runs both directions:
   pymodbus's client against salman's simulator, and salman's client against pymodbus's
   server. It needs Python and a pip install, not a service container, and the harness is
   `cargo build -p salman-modbus-net --examples`. **If no independent implementation exists
   for your protocol, say so in the ADR** — that is a real constraint and it should be
   recorded rather than skipped.

5. **A fuzz target per decoder, asserting postconditions** — not merely that nothing panicked.
   Read `fuzz/fuzz_targets/modbus_pdu.rs` before writing yours; it asserts three things:
   whatever decodes survives a round trip through the typed form; encoding is *canonical*, so
   encode-decode-encode is a fixed point (byte-identity is deliberately **not** the invariant,
   because a sender may set meaningless padding bits and salman clears them — the fuzzer is
   what established that difference); and a prefix of a frame must never decode, or the stream
   framer would hand half a frame to a caller. The fuzz workflow builds with
   `--target x86_64-unknown-linux-gnu`, pinned deliberately, and runs daily rather than per-PR.

6. **Every write behind the posture model.** As above.

7. **A trademark row in `LEGAL.md` §5**, and a plain statement that salman is not certified by
   and not affiliated with the owning organisation. Speaking a protocol and being certified
   against it are different things. `salman-modbus`'s module doc carries its own copy.

8. **Capability registry entries naming the tests**, so the protocol appears in
   `docs/STATUS.md` at the status its evidence supports and no higher.

### The honest price tag

Modbus, as it sits in the tree today: 7 242 lines counting both crates, their tests and
examples, their three fuzz targets, the interop workflow with its Python harness, and
`ADR-0012` — of which 6 609 lines are the two crates alone. It carries 133 tests, about 11% of
the workspace suite, and it arrived in seven commits totalling 8 018 insertions. Three ADRs
exist because of it — `ADR-0012` on addressing, `ADR-0013` on not taking an async runtime, and
`ADR-0014` on not driving a plant — plus its share of `salman-project`, `salman-link`,
`salman-analyse`, and `docs/CONFORMANCE.md` §26 (*"Where salman's Modbus decoder is stricter
than the specification"*) and §27 (*"The TCP overlap policy, and what a capture cannot tell
you"*). Modbus is one of the simplest fieldbus protocols in existence. Budget
accordingly, and read `docs/adr/ADR-0016-what-comes-after-modbus.md` before assuming the
second protocol is the next thing salman should build.

## The order to do it in

1. ADR. Addressing model, endianness, what the specification leaves ambiguous, whether an
   independent implementation exists for interop, and **what happens to the project file
   schema**.
2. `salman-<protocol>`: PDU types, framing per transport, function/service codes, limits, the
   device data model. Depends on `salman-core` only. Fuzz targets alongside.
3. `salman-<protocol>-net`: client and simulator. Blocking sockets. Every write takes a
   `UserConfirmation` by value.
4. Interop workflow, both roles.
5. `salman-analyse/src/<protocol>.rs`, and findings that carry evidence and say how sure
   salman is.
6. `salman-project`: the schema decision from step 1, actually implemented — and this is where
   `map.rs` gets generalised rather than duplicated.
7. `salman-link`: a way to reach a device that is not `salman_modbus_net::Client`. `Peer`
   stays. `LinkError::WouldDriveALiveDevice` stays.
8. `LEGAL.md` §5 trademark row, `LEGAL.md` §8 if a dependency arrived, capability registry
   entries, `docs/CONFORMANCE.md` for every place your decoder is stricter or looser than the
   specification and why.

Steps 6 and 7 are the ones that turn "add a protocol" into "generalise the workspace". They
are also the ones nobody estimates. Estimate them.
