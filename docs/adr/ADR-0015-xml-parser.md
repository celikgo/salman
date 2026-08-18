# ADR-0015: `xml` for PLCopen XML, and what that costs

- **Status**: Accepted
- **Date**: 2026-08-19
- **Deciders**: the salman authors

## Context

Reading PLCopen XML means parsing XML, and XML is not a format to write a parser for on the
way to somewhere else. Namespaces, entity expansion, character references, CDATA, encoding
declarations and the interaction between them are where the subtle failures live, and a
half-parser that handles the documents in front of it is exactly the sort of thing that reads
someone else's export wrongly and says nothing.

This matters more than usual here because of a property
`docs/adr/ADR-0013-no-async-runtime-yet.md` records: every crate in this workspace that
decodes bytes off a wire or out of a file salman did not write has **no dependency outside
the standard library**. A PLCopen XML file is exactly such a file.

## Decision

**`salman-plcopen` depends on `xml` 1.4.0, and that property no longer holds for every such
crate.** It still holds for the Structured Text front end, the runtime, the whole Modbus
stack and the capture reader — which is where the hostile input actually is — and it does not
hold for the PLCopen reader.

The crate was checked rather than taken on recommendation:

- **Zero dependencies of its own.** `cargo tree` shows `xml v1.4.0` and nothing beneath it.
- **`#![forbid(unsafe_code)]`** at the top of `src/lib.rs`, and the only occurrence of the
  word `unsafe` anywhere in its source is that line.
- **MIT**, which `deny.toml` already allows.
- **Billion-laughs limits are configurable**: `max_entity_expansion_length` and
  `max_entity_expansion_depth` are real fields on its reader configuration, which matters
  because an entity-expansion bomb is the attack every XML parser has to answer for.
- **It writes as well as reads.** salman has to export PLCopen XML, and two XML
  implementations would mean two divergent notions of what a document means.

`roxmltree` was rejected for the last reason alone: it cannot write.

`quick-xml` is the alternative if serde-derived deserialisation ever becomes necessary. It is
better engineered in some respects — it is the only one of the three with real fuzzing
infrastructure — but it costs `memchr`, which brings SIMD code with a great deal of `unsafe`
in it, and it would have to be pinned at **0.41.0 or later**: RUSTSEC-2026-0194 and
RUSTSEC-2026-0195, both CVSS 7.5, are patched only there.

## Consequences

**The claim in ADR-0013 has been narrowed and that ADR now says so.** "Every crate that
decodes bytes has no dependencies" was true when it was written and is not true now. Saying
which crates do and do not is more useful than a slogan, and less likely to be quietly wrong
again.

**The entity-expansion limits are set explicitly rather than left at the default**, because a
default that changes in a patch release is a limit salman did not choose.

**A fuzz target covers the reader**, as it does every other decoder here. What it can find is
narrower than for an in-crate decoder — a crash inside `xml` is not one salman can fix — but
it still covers everything salman does with what the parser returns, which is where salman's
own mistakes will be.
