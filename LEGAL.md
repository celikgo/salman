# Legal position

This file records the legal reasoning salman operates under, so that the reasoning can be
checked and argued with rather than assumed. It is not legal advice, and nobody on this
project is a lawyer.

Version 0.0.1.

---

## 1. What salman is

salman is an independent implementation, written from scratch in Rust.

It is not derived from any IEC publication. It does not reproduce the text of any IEC
publication. No part of it was produced by transcribing, translating, or mechanically
rewriting normative wording. Where salman implements behaviour that IEC 61131-3 also
specifies, the implementation was written from a description of the observable behaviour —
what goes in, what comes out, and when — and the tests assert that behaviour rather than
equivalence to a body salman does not possess.

The one place this is most visible is `crates/salman-vm/src/stdfb.rs`, the standard function
blocks. IEC 61131-3:2013 supplies no body at all for the timers: they are defined by timing
diagrams, cited as `IEC 61131-3:2013 Figure 15 "Standard timer function blocks - timing
diagrams (Rules)" (Ed 3.0)`. Every timer test in salman is therefore a trace of inputs
against outputs over virtual time.

---

## 2. Standards citation policy

salman cites IEC 61131-3 clause, table and figure **numbers** only, as references. The
purpose is that a reader who holds a licensed copy can check salman's work against the
document salman claims to follow. Every description of standard behaviour in salman's
documentation, code comments and tests is written in salman's own words.

IEC publications are copyright of the International Electrotechnical Commission, Geneva,
Switzerland, and are sold from the IEC webstore. The IEC's own statement of its copyright
terms is at <https://webstore.iec.ch/en/copyright>.

### The reasoning, and its limits

The position that citing a clause number is permissible rests on the principle that
copyright protects expression and does not extend to a procedure, process, system or method
of operation. In United States law that principle is codified at 17 U.S.C. §102(b) —
<https://www.law.cornell.edu/uscode/text/17/102>. A clause number is a locator, not
expression: it tells a reader where to look and conveys none of what the standard says.

That is where the confidence ends. **We have found no IEC statement addressing citation of
clause numbers either way.** The IEC's copyright page is about reproduction and grants no
permission and no exemption that covers citation. This is therefore a reasoned position, not
a settled one. If the IEC states a position that contradicts it, salman changes, and this
file records that it changed and why.

### The three-tier operating rule

| Tier | Rule | What it covers |
|---|---|---|
| **Do freely** | Cite clause, table and figure numbers. | Numbers and printed titles in documentation, code comments and test names, with the year and edition attached. `IEC 61131-3:2013 §7.3.3 "Statements" (Ed 3.0)`. |
| **Do carefully** | Describe, in salman's own words, the observable behaviour a test asserts, and cite the clause as the reason for asserting it. | The `requirement` field on every citation in `crates/salman-core/src/clause.rs`. Never transcribe the standard's sentence and swap words for synonyms: a close paraphrase is functionally a reproduction, and dressing a reproduction as a summary makes it worse rather than better. |
| **Never, without written permission** | Reproduce normative content verbatim. | Function block bodies, tables, figures, grammar productions, and annex examples. No exception for "short" extracts and no exception for code. |

### Things salman must not say

Three claims are convenient, unsupported, and forbidden here:

1. **That the IEC permits citation of clause numbers.** Their published policy is silent on
   it. Silence is not permission, and citing silence as permission would be exactly the kind
   of confident overreach this project exists to avoid.
2. **That short extracts are fair use.** No source we have supports that as a general
   proposition, fair use is a defence decided case by case rather than a licence, and the
   doctrine is jurisdiction-specific.
3. **That *ASTM v. Public.Resource.Org* covers salman.** It does not. That holding is
   limited to standards that have been incorporated by reference into law, and it leaned on
   the defendant's nonprofit public-access purpose. IEC 61131-3 is not incorporated into law
   in the jurisdictions salman is developed in, and salman is not a nonprofit publisher of
   the law. Neither half applies, and citing the case as cover would be borrowing an
   argument that was never about us.

---

## 3. Edition

salman targets **IEC 61131-3:2013 (Edition 3.0)**.

Edition 3.0 was withdrawn on 2025-05-22 and superseded by IEC 61131-3:2025 (Edition 4.0),
which is the current edition. salman targets Edition 3.0 because that is the edition its
public sources allow it to verify. No Edition 4.0 normative text has been read by anyone on
this project, so salman makes no Edition 4.0 claim of any kind — not conformance, not
partial support, not "mostly the same".

Clause numbers are edition-specific, and this is not a pedantic point. Structured Text is
§7.3 in Edition 3.0 and §7.2 in Edition 4.0, and the standard function block tables shift
between the two. A citation without an edition therefore identifies nothing. Every citation
salman publishes carries the year and the edition; `docs/IEC_CITATIONS.md` is generated from
the registry in `crates/salman-core/src/clause.rs` and cannot drift away from it.

Product pages:

- IEC 61131-3:2025 (Edition 4.0) — <https://webstore.iec.ch/en/publication/68533>
- IEC 61131-3:2013 (Edition 3.0), withdrawn —
  <https://webstore.iec.ch/en/publication/4552>

---

## 4. Safety

salman is **not** certified, assessed, qualified or approved under IEC 61508, IEC 62061,
ISO 13849, or any other functional safety standard. No such assessment is under way, and
none is planned.

A tool that generates outputs which can directly or indirectly contribute to the executable
code of a safety-related system is, in the vocabulary of `IEC 61508-3:2010 "Functional
safety of electrical/electronic/programmable electronic safety-related systems - Part 3:
Software requirements" (Ed 2.0)` (<https://webstore.iec.ch/en/publication/5517>), an
off-line support tool, and such a tool requires qualification evidence before it may be
used in that role.
salman is such a tool. No qualification evidence exists for it.

Therefore:

- Do not use salman to develop, generate, modify, verify or validate any safety function,
  any safety-related control system, or any safety-related part of a control system. That
  includes `IEC 62061:2021 "Safety of machinery - Functional safety of safety-related
  control systems" (Ed 2.0)` (<https://webstore.iec.ch/en/publication/59927>) and
  ISO 13849, whose product page is not linked here because the ISO site does not serve the
  automated link checker this repository runs.
- Do not use salman output on any machine or process where a failure could cause injury,
  death, environmental harm, or damage to property.

salman produces no evidence usable in a functional safety argument. Its test suite is a
correctness suite. It exists to catch salman's own mistakes, not to demonstrate systematic
capability, and reading a green test run as safety evidence would be reading it as something
it was never constructed to be.

**This is a limitation of the tool, not merely a disclaimer of liability.** The distinction
matters. A disclaimer says who bears the loss when something goes wrong; the statement above
says salman is missing the artefacts that would make its use in this role defensible in the
first place. The warranty disclaimer and limitation of liability in Apache-2.0 §§7-8 apply
in addition to this, not instead of it.

---

## 5. Trademarks

salman is an independent open-source project. It is not affiliated with, endorsed by, or
sponsored by any third party named below, or by the IEC.

| Mark | Owner |
|---|---|
| Beckhoff, TwinCAT, EtherCAT, Safety over EtherCAT | Beckhoff Automation GmbH & Co. KG |
| CODESYS | CODESYS GmbH |
| Studio 5000, Logix, ControlLogix, CompactLogix, RSLogix | Rockwell Automation, Inc. |
| PROFIBUS, PROFINET | PROFIBUS Nutzerorganisation e.V. / PROFIBUS & PROFINET International |
| EtherNet/IP, CIP, DeviceNet | ODVA, Inc. |
| OPC UA, OPC Foundation | OPC Foundation |
| CANopen | CAN in Automation |
| PLCopen | PLCopen |
| Modbus | its owner — see below |

All other trademarks are the property of their respective owners.

### Why the Modbus row names nobody

It would be easy to write "Modbus is a trademark of Schneider Electric" and it is written
that way in a great many places. We could not establish it from a primary source. The legal
notice published by modbus.org lists Schneider Electric marks, and "Modbus" is not among
them. That page is not linked here because it refuses the automated link checker this
repository runs, so a reader should look it up rather than take our word for the contents.

Naming the wrong owner of a mark in a legal file is a worse error than declining to name
one, so the hedge stays until a primary source settles it. This is the same rule the
citation registry follows for clause numbers it could not confirm: state the uncertainty in
the record instead of resolving it by guessing.

### Phrasing rules

Safe, because each is a factual statement about what salman reads or aims at:

- "salman imports Rockwell L5X files"
- "salman reads the PLCopen XML exchange format"
- "based on the OPC UA specifications"
- "aims at IEC 61131-3:2013 Structured Text; see `CONFORMANCE.md`" — `docs/CONFORMANCE.md`
  exists and states, feature by feature, what is implemented, what is tested, what is
  absent, and what salman decided for itself

Never, because each asserts a relationship or an approval that does not exist:

- "Rockwell-compatible", or any vendor name joined to "-compatible"
- "certified", "conformant", "compliant", "approved", "endorsed" or "official" applied to
  salman in connection with any vendor, any trade association, or the IEC
- "IEC 61131-3 compliant", full stop, with or without qualification
- vendor logos, wordmarks, or brand colours anywhere in the project
- any product name or crate name containing a third-party mark

At 0.0.1 salman implements none of those formats or protocols. The names above do appear
elsewhere in the repository: in `README.md`, `docs/ROADMAP.md`, `docs/CONFORMANCE.md`,
`docs/STATUS.md`, `docs/adr/ADR-0003-plcopen-xml-canonical.md` and
`docs/adr/ADR-0007-dialects.md`, and in source comments and diagnostic text in
`crates/salman-lang` and `crates/salman-vm` where a dialect divergence is named — CODESYS
and Beckhoff on the binding strength of `**`, for instance. In every case the name is used
as a factual statement about what another tool does or about work not yet started, and
never joined to salman in any of the forms forbidden above.

---

## 6. Licence

salman is licensed under Apache-2.0. The full, unmodified licence text is in `LICENSE` at
the repository root; the canonical text is at <https://www.apache.org/licenses/LICENSE-2.0>
and the SPDX identifier is `Apache-2.0` (<https://spdx.org/licenses/Apache-2.0.html>).

- Every workspace member declares `license.workspace = true`, which resolves to
  `license = "Apache-2.0"` in the root `Cargo.toml`. The fuzzing crate declares it
  literally.
- Source files carry an `SPDX-License-Identifier: Apache-2.0` line. This is **not yet
  uniform**: at 0.0.1, 31 of 48 source files carry one and the rest do not. That is a defect
  in the tree, it is recorded here rather than described as finished, and it is closed by
  adding the missing lines, not by softening this paragraph.
- There is no `NOTICE` file at 0.0.1. Apache-2.0 §4(d) is conditional — it obliges a
  redistributor to carry forward a `NOTICE` that exists — and salman vendors no
  Apache-licensed dependency that ships one. A `NOTICE` file will be added if and when
  a dependency arrives that requires it.
- Apache-2.0 §6 grants no trademark rights, in either direction. The licence gives nobody
  permission to use the name "salman" as a mark, and it gives salman no rights in anybody
  else's mark.

### Open question: the copyright holder line

**The copyright holder has not been settled, and this file deliberately does not assert
one.**

Whether an author's employment terms affect ownership of work written outside employment is
jurisdiction-specific and contract-specific, and it is not a question that can be answered
by reading the repository. It must be resolved before the first public release, because a
copyright line asserted wrongly is harder to unwind afterwards than one that was never
written. Until it is resolved, salman ships under Apache-2.0 with no holder line, and this
paragraph is the record of why.

---

## 7. Export control

Recorded here as a checked non-issue, so that nobody has to re-litigate it from scratch.

salman is published as open source, in a public repository, with unlimited distribution and
no restriction on who may obtain it. Under 15 CFR §734.7
(<https://www.law.cornell.edu/cfr/text/15/734.7>), unclassified software that is made
publicly available without restrictions on its further dissemination is "published", and
published software is not subject to the Export Administration Regulations.

Two notes on the edges of that:

- §734.7 carves out published encryption software, which remains controlled. salman
  implements no cryptography. It contains a SHA-256 implementation used as a content
  fingerprint for simulation traces, which is a hash and not encryption, and which
  `crates/salman-core/src/hash.rs` explicitly documents as not a security primitive. If
  salman ever ships cryptography, this paragraph is the one to revisit first.
- The cybersecurity items rule is scoped to intrusion software and to carrier-class network
  surveillance products. A PLC engineering platform is neither. salman additionally refuses,
  in code and at every posture, the categories of behaviour that rule is aimed at — see
  `SECURITY.md` and `crates/salman-core/src/posture.rs`.

Nothing here is legal advice, and this analysis considers United States law only. Other
jurisdictions have their own regimes and salman has not assessed them.

---

## 8. Upstream licences

**salman vendors no third-party protocol stack at 0.0.1.** There is no fieldbus code, no
OPC UA code, no network code of any kind in this repository. There are exactly two direct
third-party dependencies — the command-line argument parser and the YAML reader for
declarative test files — and everything else in the graph is pulled in by one of those two.
`salman-core`, `salman-lang` and `salman-vm` have no third-party dependency at all.

| Dependency | Role | Licence | How it enters |
|---|---|---|---|
| `clap`, with `clap_builder`, `clap_lex` and `anstyle` | Command-line argument parsing for `salman-cli` | MIT OR Apache-2.0 | crates.io, direct dependency of `salman-cli`, pinned in the committed `Cargo.lock` |
| `serde` and `serde_core` | Deserialising the declarative test-file schema in `salman-test` | MIT OR Apache-2.0 | crates.io, direct dependency of `salman-test` |
| `serde-saphyr` | The YAML reader behind `.salman-test.yaml` files | MIT OR Apache-2.0 | crates.io, direct dependency of `salman-test` |
| `annotate-snippets`, `unicode-width` | Error snippet rendering, via `serde-saphyr` | MIT OR Apache-2.0 | crates.io |
| `granit-parser`, `arraydeque`, `smallvec`, `nohash-hasher`, `num-traits`, `base64`, `zmij` | Scanning, containers and number and byte-string conversion, via `serde-saphyr` | MIT OR Apache-2.0, except `zmij`, which is MIT only | crates.io |
| `encoding_rs`, `encoding_rs_io`, `cfg-if` | Character-encoding detection on a test file, via `serde-saphyr` | MIT OR Apache-2.0, and `encoding_rs` additionally BSD-3-Clause for the encoding tables | crates.io |
| `clap_derive`, `serde_derive`, `proc-macro2`, `quote`, `syn`, `heck` | Build-time macro support for the two derive macros above | MIT OR Apache-2.0 | crates.io, procedural macro and build dependencies only |
| `unicode-ident` | Identifier character classes, via `syn` | MIT OR Apache-2.0, plus Unicode-3.0 for the character data | crates.io |
| `autocfg` | Compiler feature detection, build script of `num-traits` | Apache-2.0 OR MIT | crates.io, build dependency only |

Every one of those is redistributable under Apache-2.0 terms. The allowlist that enforces it
lives in `deny.toml` at the repository root, and `cargo-deny` runs it in CI: a licence the
project has not considered fails the build rather than entering quietly.

### The forward-looking warning

This is the part worth reading before the first protocol lands.

Several of the open implementations salman would most want to test against — IEC 61131-3
compilers, soft-PLC runtimes, and industrial protocol stacks — are published under GPL-3.0
or LGPL-3.0. Neither can be vendored into an Apache-2.0 binary without compliance work that
nobody has done, and in the GPL-3.0 case the result would not be distributable under
Apache-2.0 terms at all.

The rule salman adopts, so that this does not get decided under deadline pressure later:
such a project may be used as an **external differential-testing oracle, run as a
subprocess**, with its output compared against salman's. It is never linked, never vendored,
and never a build dependency. That keeps the licences separated by a process boundary and
keeps the comparison honest, because an oracle salman links against is an oracle salman can
accidentally agree with for the wrong reason.
