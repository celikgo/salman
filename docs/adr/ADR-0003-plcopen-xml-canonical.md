# ADR-0003: PLCopen XML as the canonical interchange form

- **Status**: Accepted (not yet implemented)
- **Date**: 2026-08-18
- **Deciders**: the salman authors

## Context

salman implements no importer at version 0.0.1. There is no code in this repository that
reads an L5X file, a `.TcPOU` file, a Siemens SCL export or a PLCopen XML document. This
ADR records a decision about the shape of work that has not started, so that the first
importer is written against a settled answer rather than settling it by accident.

The forces are these. An engineering platform that claims to be vendor-neutral has to read
what vendors actually produce, and every vendor produces something different: Rockwell
exports L5X, Beckhoff writes `.TcPOU` and `.TcPRO`, Siemens exports SCL as text, and a
great deal of code exists only as plain Structured Text files with no project structure
around them. Each of those formats encodes not only the code but a model of what a project
is, and those models disagree.

There is exactly one neutral candidate. PLCopen's XML exchange format was developed by its
TC6 working group for exactly this purpose; see
[PLCopen's page on it](https://www.plcopen.org/standards/xml-echange/) — the missing "x" in
that URL is genuinely how it is spelled, and `xml-exchange` does not exist. It is not a
perfect model of every vendor's project, and no such model exists.

**An earlier version of this paragraph said that in 2019 the format "was adopted into the
standard as IEC 61131-10:2019", which reads as continuity and is wrong.** They are two
different formats. PLCopen's own flyer states: *"This new version is not compatible to
previous versions of PLCopen XML."* Structurally they share almost nothing — different
namespace, `<project>` against `<Project>`, lowercase against PascalCase, three kinds of POU
against nine, and, most consequentially, Structured Text stored as XHTML-wrapped markup in
one and as a plain string in the other. §"Which PLCopen XML" below records which salman
targets.

The other force is honesty about fidelity. "Imports Rockwell projects" is a claim that
means nothing without a statement of what survives the trip. Every tool in this space makes
that claim; very few publish what is lost.

Note on editions: salman's language work targets IEC 61131-3:2013 (Edition 3.0), which was
[withdrawn on 2025-05-22](https://webstore.iec.ch/en/publication/4552) and superseded by
IEC 61131-3:2025 (Edition 4.0). salman targets Edition 3.0 because it is the edition our
public sources let us verify. IEC 61131-10:2019 is a separate part of the standard,
unaffected by that withdrawal.

## Which PLCopen XML

**salman targets PLCopen XML v2.01**, the current PLCopen format: "XML Formats for
IEC 61131-3", Official Release, 2009-05-08, schema `tc6_xml_v201.xsd`, target namespace
`http://www.plcopen.org/xml/tc6_0201`.

The reasons, all checkable:

- **It is what tools actually write.** Every vendor export examined — CODESYS, TwinCAT,
  Rexroth ctrlX, WAGO, Schneider, Beremiz, OpenPLC — is v2.01 or v2.0.
- **It is free and it is stable.** The schema downloads without a login, and the copy
  archived in 2017 from an entirely different URL is byte-identical to today's: SHA-256
  `591b92ba65018a77c32ab9e606abf27bd810cb1f7761d972a85689531c51e20f`.
- **Its type set matches what salman implements.** v2.01 is frozen at IEC 61131-3 2nd
  edition and rejects `LTIME`, `LDATE`, `LTOD`, `LDT`, `CHAR` and `WCHAR` — which are
  exactly the types salman does not implement either.
- **There is no v3.0.** The full downloads listing contains no schema or document above
  2.01, and PLCopen's own page says "The latest PLCopen version is 2.01". Anything
  describing a "PLCopen XML 3.0" is most likely confusing it with IEC 61131-3 **3rd
  edition**. salman claims no more than: no v3.0 is published by PLCopen as of 2026-08-19.

**IEC 61131-10:2019 is a separate future target, not a migration.** It is paywalled at
CHF 475, and its clause 5 — which is what a project would need to describe its own
compliance posture — is behind that wall: the free 26-page preview stops after clause 3. So
salman will not describe an IEC 61131-10 compliance posture at all until somebody has read
clause 5.

**salman will not ship a copy of either schema.** For v2.01 that is a firm finding rather
than a gap: the document carries no licence or terms-of-use statement, and no redistribution
grant could be found anywhere on PLCopen's site. Not finding permission is not permission.
salman reads the format from a schema the user supplies or from its own model of the format,
and cites the schema rather than carrying it.

**salman will not claim conformance or certification.** PLCopen runs no conformance or
certification programme for the XML format. It certifies Logic, Motion Control, Safety and
Training Centers, and certification is for voting members only. The XML side has a
members-only logo scheme whose own text says a certification document "is currently under
construction" — that was 2009, and no such document is on the downloads page today.

## The one thing that will surprise an implementer

**In v2.01, Structured Text is not stored as text.** `<ST>` has type `formattedText`, whose
whole definition is a sequence of exactly one element from the XHTML namespace with
`processContents="lax"`. So `<ST>a := TRUE;</ST>` does not validate, and neither does a bare
`CDATA` section: the code has to be inside an XHTML element.

The specification does not say **which** element, contains no worked ST example anywhere in
its eighty numbered pages, and imports no XHTML schema — so the namespace is constrained and
the element name is not. Real tools have split into two families as a result:

| Family | What it writes |
|---|---|
| CODESYS, TwinCAT, ctrlX, WAGO, Schneider | `<ST><xhtml xmlns="http://www.w3.org/1999/xhtml">…</xhtml></ST>` — an element named `xhtml`, which does not exist in XHTML 1.1 |
| Beremiz, OpenPLC Editor | `<ST><xhtml:p><![CDATA[…]]></xhtml:p></ST>` |

Both validate. **A reader that keys on the element name fails on half the ecosystem**, so
salman accepts any single element in the XHTML namespace and takes its text content. This is
under-specification in the standard rather than vendors misbehaving, and there is no correct
answer to look up.

## Decision

When importers arrive, salman adopts PLCopen XML as its canonical internal interchange
form. Every other format is imported *into* it, and every export goes *out of* it. There is
one hub and no pairwise paths.

Specifics:

- Rockwell L5X, Beckhoff `.TcPOU`, Siemens SCL text and plain ST files are all sources that
  produce PLCopen XML. Nothing else in salman consumes a vendor format directly.
- A lossy import is a first-class outcome, not an error and not a silent success. Anything
  the source expresses and PLCopen XML cannot is recorded as unrepresentable, named, and
  reported to the user — never approximated into the nearest construct that happens to
  parse.
- Round-trip fidelity is never claimed without a test. `docs/COMPATIBILITY.md` will be
  generated by CI from real round-trip tests over golden projects, with a per-construct
  pass or fail and an explicit list of what is lost in each direction. It will be generated,
  never written by hand.
- salman will not depend on TIA Portal Openness. It is a [.NET API that drives an installed
  TIA Portal engineering
  system](https://assets.new.siemens.com/siemens/assets/api/uuid:0fdd52a4-c384-4e55-a89d-ba9181d17fc7/tia-openness.pdf).
  Depending on it would mean an open-source tool required a commercial licence to build and
  to test, and would tie a binary that is meant to run on Linux, macOS and Windows to
  Windows alone. Siemens projects are therefore imported from exported text, with whatever
  reduction in fidelity that entails, and that reduction goes in the compatibility table
  like any other.

## Consequences

Everything pays the hub tax. Importing an L5X file to compare two Rockwell projects goes
through PLCopen XML and back, and any construct the hub cannot express is degraded in a
comparison between two files that both expressed it perfectly. This is the direct cost of
neutrality and it is not recoverable by clever engineering.

PLCopen XML cannot express everything every vendor does. Vendor-specific attributes,
proprietary function blocks, hardware configuration and IDE metadata mostly have no place
in it. Those become unrepresentable records: preserved as opaque annotations where that is
possible, reported and dropped where it is not.

A lossy import must be reported, which means the import path needs a channel for saying so
and the user interface needs somewhere to show it. That is more work than a boolean success
and it will make salman look worse than tools that smooth the same losses over in silence.
This is intended. A tool that silently approximates a construct produces a project that
looks correct and is not.

Generating `docs/COMPATIBILITY.md` from tests means the compatibility story cannot be
better than the golden corpus. A construct nobody wrote a golden project for is a construct
with no row in the table, and the table will need to say that rather than imply full
coverage.

Excluding TIA Portal Openness costs real fidelity on Siemens projects, which are a large
share of the installed base in Europe. Some of what Openness exposes is not in any text
export. salman will simply be worse at Siemens projects than a Windows tool that uses
Openness, and saying otherwise later would be a reversal of this decision, not a
refinement of it.

## Alternatives considered

**salman's own JSON or TOML interchange format.** Straightforward to design, pleasant to
diff, and it could express exactly what salman's model needs with no impedance mismatch.
It lost because nobody else can read it. The point of a canonical interchange form is
neutrality, and a format defined by one tool is not neutral — it is that tool's internal
representation with a file extension. It would also put salman in the position of asking
vendors to support a format salman invented, which is not a position a pre-alpha project is
in.

**Direct pairwise converters between vendor formats.** Highest possible fidelity for each
pair, because nothing has to survive a hub. It lost on arithmetic. Every new format has to
be taught about every existing format, which is N-squared work and N-squared test surface,
and the quality of any given pair depends on whoever last touched it. It also has no
natural place to state what is lost, because the loss is different for every pair.

**Using one vendor's format as canonical.** Attractive because those formats are richer
than PLCopen XML and are backed by working implementations. It was rejected because
adopting a vendor's project model as the internal model is vendor lock-in with extra steps:
every other vendor's constructs then get expressed in one vendor's vocabulary, and salman's
neutrality claim becomes a marketing sentence.

**No canonical form at all — model each format separately in the tool.** This is what
several existing tools do. It lost because every feature that operates on a project would
then have to be written once per format, and features that compare projects across vendors,
which is a large part of why salman exists, would have nowhere to stand.

## How this is enforced

Nothing enforces this today, and this ADR should not pretend otherwise. There is no
importer, no golden project corpus, no `compat.yml` workflow and no
`docs/COMPATIBILITY.md`. The decision is currently held in place by this document and by
review.

The enforcement arrives with the first importer, at v0.2, and consists of two things:

- `.github/workflows/compat.yml`, which runs round-trip tests over the golden project
  corpus on every push and fails when a construct's recorded fidelity changes.
- `docs/COMPATIBILITY.md`, generated by that job from the test results, per construct and
  per direction, listing explicitly what is lost. Rule 9 in `README.md` already states the
  governing principle: compatibility claims are generated by CI, never written by hand.

Until both exist, salman must not make a compatibility claim about any vendor format in
any document, including this one.
