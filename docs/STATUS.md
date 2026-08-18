# salman capability status

*Generated from `salman_core::capability::REGISTRY`. Do not edit by hand.*

`[x]` implemented and tested  ·  `[~]` implemented, untested  ·  `[-]` stub  ·  `[ ]` planned

A capability is only marked *implemented and tested* if it names tests that exist in this repository. A test in this crate checks that they do.

## Determinism

| | Capability | Status | Milestone | Notes |
|---|---|---|---|---|
| `[x]` | Seeded xoshiro256++ generator, pinned and recorded in every trace header | implemented and tested | v0.1 | Written out in-crate rather than taken from rand, whose StdRng and SmallRng are documented as non-portable. Not cryptographic: never use it for a key or a token. |
| `[x]` | In-crate SHA-256 fingerprint of simulation traces, with NIST known-answer tests | implemented and tested | v0.1 | A content fingerprint, not a security primitive: not constant-time, and not to be used where an attacker picks the input and the comparison is secret. Written in-crate so there is no runtime CPU-feature dispatch and no C toolchain. |

## Language

| | Capability | Status | Milestone | Notes |
|---|---|---|---|---|
| `[x]` | Diagnostics with spans, IEC clause citations and the dialect rule applied | implemented and tested | v0.1 | Rendered in plain text with no colour, so meaning never depends on colour and golden tests can compare bytes. |
| `[x]` | Case-insensitive, case-preserving IEC identifiers | implemented and tested | v0.1 | ASCII case rules only, so identifier identity cannot shift with a Unicode version. |
| `[x]` | Source map, spans and line/column resolution for diagnostics | implemented and tested | v0.1 | Source files above 64 MiB are refused rather than loaded. |
| `[x]` | TIME, LTIME, DATE, TIME_OF_DAY and DATE_AND_TIME values | implemented and tested | v0.1 | Leap seconds, time zones and daylight saving are not modelled: every day here is exactly 86 400 s. |
| `[x]` | Elementary types, the ANY generic hierarchy, and runtime values | implemented and tested | v0.1 | CHAR, WCHAR, LDATE, LTOD and LDT are not implemented. NaN is canonicalised on entry so that a trace cannot differ between architectures. |
| `[x]` | Several source files build as one program | implemented and tested | v0.2 | Files are parsed from disjoint node-id ranges and joined before checking, so a name declared twice across two files is a duplicate rather than two valid declarations. `salman check` and `salman run` take several paths; `salman test` still takes one, and there is no project file yet. |
| `[x]` | Dialects as configuration, with every diagnostic naming the rule it applied | implemented and tested | v0.1 | Two profiles ship: generic and iec61131-3:2013-strict. No vendor profile exists, and DialectId does not contain one. |
| `[x]` | EN and ENO on every call, as part of the calling convention | implemented and tested | v0.1 | EN and ENO are not declared by a POU, so no POU may declare a variable of either name. EN on a call whose result is used is refused: with EN false there is no call and therefore no result, and salman will not invent one. |
| `[~]` | libFuzzer targets for the Structured Text front end, asserting its postconditions | implemented, untested | v0.1 | Six of the nine targets in fuzz/fuzz_targets; the other three cover the Modbus decoders and have their own entry. Four cover the lexer: valid UTF-8, raw bytes decoded the way the loader will decode them, the strict dialect, and a differential run of both dialects. One covers the parser, and one covers lexing, parsing and semantic analysis together. Each asserts what must hold for any input — exactly one Eof, non-decreasing spans inside the source, every literal and address index resolving, every node id usable as an index into a side table — rather than only that nothing panicked. All six build and run under nightly, and .github/workflows/fuzz.yml runs every target it finds for 60 s daily. Not ImplementedTested, for two reasons that both matter: a fuzzing run shows that nothing was found, which is not the same as showing anything is right, and this registry's evidence rule wants a named test function, which a libFuzzer target is not. The declarative test-file reader in salman-test is not covered. |
| `[x]` | Recursive-descent Structured Text parser with error recovery and bounded nesting | implemented and tested | v0.1 | Every statement and declaration form of Structured Text, with the Edition 3 operator precedence: unary binds tighter than `**`, so `-2 ** 2` is 4, and salman warns where CODESYS and Beckhoff would give -4. Errors produce error nodes and resynchronise rather than stopping the parse. Nesting, including left-associative operator chains, is bounded by the dialect. Three things are salman rules rather than verified requirements and say so in the diagnostic: duplicate and overlapping CASE labels are refused, a FOR body may not assign to its control variable, and the value of that variable after the loop is unspecified. Inline structures and enumerations, VAR_CONFIG instance paths, single-resource configurations, references and the object-oriented extensions are parsed far enough to be named and are not implemented. |
| `[x]` | Name resolution, type checking, constant folding and recursion rejection | implemented and tested | v0.1 | Rejecting recursion is what makes the compiler's single-static-frame layout sound. The prohibition itself is a salman rule: it is widely attested but the clause could not be verified, and the diagnostic says so. Three constructs are resolved and then refused rather than half-implemented: references, the assignment attempt, and VAR_EXTERNAL. |

## Project infrastructure

| | Capability | Status | Milestone | Notes |
|---|---|---|---|---|
| `[x]` | Generated capability status, with tests cited as evidence | implemented and tested | v0.1 | Status tables in the README and docs are generated from this registry. |
| `[x]` | IEC clause citation registry with explicit provenance | implemented and tested | v0.1 | 43 citations are registered — 22 clauses, 18 tables and 3 figures of IEC 61131-3:2013 (Edition 3.0) — each with a number cross-checked against a public source and a requirement paraphrased in salman's own words. docs/IEC_CITATIONS.md is generated from the registry and cannot drift from it. A citation being registered does not mean the behaviour it names is implemented. |
| `[x]` | One source of version truth, checked when the crate compiles | implemented and tested | v0.1 | The root VERSION file and Cargo's version cannot disagree: the mismatch is a compile error, not a CI job. |

## Protocols

| | Capability | Status | Milestone | Notes |
|---|---|---|---|---|
| `[x]` | A project file binding a device's registers to the process image | implemented and tested | v0.2 | The mapping is declared in a file rather than in code, so the person who knows the plant can read it. Which way data moves is not a key the file can set: %I is read from the device and %Q is written to it. Widths must agree, ranges must exist, no two mappings may claim the same image bits, and a misspelt key is refused rather than ignored — an ignored one would leave a program reading zeros from an input it believed was live. Nothing yet runs a mapping: this reads and checks one. |
| `[x]` | A Modbus TCP client and a simulator, over real sockets, with writes gated | implemented and tested | v0.2 | Real TCP on loopback, not a mock. Blocking sockets and a thread per connection; salman has no async runtime and no dependency on one, see docs/adr/ADR-0013-no-async-runtime-yet.md. A read needs no permission. A write to a real device needs the ARMED posture and a UserConfirmation taken by value, so it authorises exactly one call. A response whose transaction identifier matches nothing outstanding is counted and skipped rather than returned as the answer to the question being asked. |
| `[~]` | libFuzzer targets for every Modbus decoder, asserting their postconditions | implemented, untested | v0.2 | Three targets: the protocol data unit, the TCP stream framer and the serial frame. Each asserts a property rather than the absence of a crash — that encoding what was decoded is a fixed point, that no prefix of a frame decodes, that what the framer delivers does not depend on where the segments were cut, and that the CRC catches every single-bit error. The framer target also asserts progress, because a framer that returned a frame without consuming input would hang every caller rather than crash. The fixed-point property is there because the fuzzer found the naive byte-identity claim false in seconds: 0F 04 01 00 04 01 FD sets padding bits salman deliberately clears. Not ImplementedTested for the same two reasons as the front-end targets: finding nothing is not evidence of correctness, and this registry wants a named test function. |
| `[x]` | A Modbus server's data model and the exceptions it answers with | implemented and tested | v0.2 | Four tables, each a declared range rather than a full 65536 items, so that exception 02 — an address outside the map — is something salman can actually produce. A multi-register write that fails validation changes nothing. The order of the checks is salman's decision: APS Figure 9 and the per-function figures of §6 disagree, and salman follows §6. Nothing here opens a socket; this decides the answer and something else will carry it. |
| `[x]` | Modbus TCP stream framing and RTU serial frames | implemented and tested | v0.2 | TCP: frames are reassembled from a byte stream, and what comes out does not depend on where the segments were cut. A bad length or a non-zero protocol id is fatal to the connection, because a Modbus TCP stream carries no sync word and resynchronising would mean guessing. RTU: an ADU with its CRC, and the timing rules that delimit one — but a byte stream alone cannot be framed as RTU, and salman says so rather than pretending otherwise. |
| `[x]` | Modbus protocol data units, decoded and encoded, with no allocation | implemented and tested | v0.2 | Eight function codes: read and write, bits and words. The rest decode by number and are reported as not implemented rather than guessed at. Nothing here opens a socket or reads a file — no transport, no client and no server exists yet. Addresses are the PDU addresses on the wire; salman applies no 4xxxx offset anywhere, see docs/adr/ADR-0012-modbus-addressing.md. |

## Runtime

| | Capability | Status | Milestone | Notes |
|---|---|---|---|---|
| `[x]` | AT %IX0.0 binds a variable to the process image, with no copy | implemented and tested | v0.2 | A located variable IS its location: it has no slot, so it cannot go stale. The declared width must match the address size, and a program may not write its own inputs. Nothing yet maps a device's registers onto the image; that is the Modbus layer. |
| `[x]` | Bytecode compiler with static instance layout and no run-time allocation | implemented and tested | v0.1 | Exponentiation and VAR_EXTERNAL are reported as not implemented rather than compiled to something approximate. Every value stored into a declared destination passes through one coercion point, so a subrange bound or a string length cannot be enforced at one site and forgotten at another. |
| `[x]` | Subrange bounds and string lengths enforced wherever a value is stored | implemented and tested | v0.1 | A subrange violation is a fault naming the variable, the value and the bounds; a string too long for its target keeps the characters that fit, which is what the standard defines. Both are salman decisions where the standard is silent. |
| `[x]` | Bytecode interpreter that faults rather than panics, with a scan watchdog | implemented and tested | v0.1 | Integer overflow wraps and division by zero faults; both are salman decisions, documented in docs/CONFORMANCE.md. |
| `[x]` | Scan semantics with a correct process image, and a visible force list | implemented and tested | v0.1 | A located variable and a directly represented variable in an expression both reach the image. Nothing maps a device's registers onto it yet. |
| `[x]` | RETAIN and PERSISTENT across simulated warm and cold restarts | implemented and tested | v0.1 | The runtime models it; no command line surface exposes a restart yet. |
| `[x]` | Cyclic, event and freewheeling tasks with priority and overrun detection | implemented and tested | v0.1 | Pre-emption is NOT modelled: a scan is atomic. A race that depends on being interrupted mid-scan cannot be reproduced here. |
| `[x]` | All ten IEC standard function blocks, with their awkward edge cases asserted | implemented and tested | v0.1 | SEMA is also provided and is NOT an IEC standard function block; see docs/CONFORMANCE.md. |
| `[x]` | Virtual clock, so a ten-minute sequence tests in milliseconds, identically | implemented and tested | v0.1 | A real-time mode exists in the type and reports its measured jitter; nothing drives it yet, because there is no hardware to be in the loop with. |

## Safety

| | Capability | Status | Milestone | Notes |
|---|---|---|---|---|
| `[x]` | OBSERVE / SIMULATE / ARMED posture model with categorical refusals | implemented and tested | v0.1 | The first caller exists. salman_modbus_net::Client::write takes a UserConfirmation **by value**, so one confirmation authorises one write and cannot be kept and reused; that type has no public constructor and can only come from asking a person, so an agent cannot manufacture consent. The posture is checked at the write as well as by whatever called in, because a check a caller can forget is not a boundary. Running a simulator needs SIMULATE: salman refuses to start one while observing. |

## Testing

| | Capability | Status | Milestone | Notes |
|---|---|---|---|---|
| `[x]` | Declarative unit tests for POUs, on a virtual clock, with no vendor runtime | implemented and tested | v0.1 | One source file per run: `salman test` takes two positional paths already, so a list of sources waits for the project file. `salman check` and `salman run` build several files as one program. |
| `[x]` | Golden-trace tests against a reviewable text file | implemented and tested | v0.1 | --update-golden rewrites them. Read the diff before committing it. |
| `[x]` | JUnit XML report and a real exit code, for a build server | implemented and tested | v0.1 | Targets the Jenkins junit-10 schema, the strictest of the three in circulation. |
