# ADR-0012: Modbus addresses are the PDU addresses, and nothing else

- **Status**: Accepted
- **Date**: 2026-08-18
- **Deciders**: the salman authors

## Context

There are two ways to write a Modbus address, and one of them is not in the specification.

**The PDU address** is what travels on the wire: a 16-bit number, `0x0000` to `0xFFFF`,
carried in the request exactly as written. MODBUS Application Protocol Specification
V1.1b3 §4.3 and §4.4 define it and nothing else.

**The `4xxxx` convention** — `40001` for the first holding register, `30001` for the first
input register, `10001` for the first discrete input, `0xxxx` for coils — is Modicon usage
that predates the open specification and that a great deal of vendor documentation still
uses. Under it, holding register `40001` is PDU address `0`, and `40108` is PDU address
`107`.

The two differ by one, by a table prefix, or by both, and which of those applies depends on
the vendor whose manual is open. This is the single most common source of a wrong reading
in Modbus work: the value comes back, it is plausible, and it is the wrong register.

The decisive fact is checkable. The strings `40001`, `4xxxx`, `4x`, `3xxxx`, `30001` and
`10001` appear **zero times** across the full text of all four Modbus Organization
documents salman consulted:

- MODBUS Application Protocol Specification V1.1b3, 26 April 2012
- MODBUS over Serial Line Specification and Implementation Guide V1.02, 20 December 2006
- MODBUS Messaging on TCP/IP Implementation Guide V1.0b, 24 October 2006
- MODBUS over Serial Line Specification and Implementation Guide V1.0

The nearest thing the specification sanctions is its own data-model numbering, which counts
items from 1 and then states the rule explicitly: a Modbus datum numbered X is addressed in
the PDU as X−1 (APS §4.4). That is a numbering of the model, not a `4xxxx` address, and it
is off by one from the wire by design.

## Decision

**On the wire, salman always uses the PDU address, and applies no transformation of any
kind.**

**To the user, salman shows the PDU address**, in every command-line argument, every
configuration field, every capture column, every timeline label and every diagnostic. The
mapping between what a user types and what goes on the wire is the identity. There is no
offset to get wrong, because salman does not apply one.

Any column or label that carries an address is named so that its convention is visible —
`pdu_addr`, or `addr (PDU, 0-based)` — and never a bare `address`, which would leave the
reader to assume.

If salman ever offers a `4xxxx` view, it will be a **presentation adapter** and will obey
four rules: it is opt-in and never inferred; every value it renders is tagged with the PDU
address it means, as `40001 [pdu 0]`; the documentation states that the convention appears
in none of the four documents above; and no code below the presentation boundary ever sees
a non-PDU address. It is not implemented, and the roadmap does not promise it.

## Consequences

**A user reading a vendor manual that says `40108` has to subtract.** That is a real cost
and it falls on the person salman is meant to help. It is accepted because the alternative
is worse: a tool that silently applies an offset is right for the vendors whose convention
it guessed and wrong for the rest, and the wrongness looks like a working system returning
a plausible number. salman's diagnostics name the convention so that the subtraction is at
least an informed one.

**Two devices that disagree about the convention cost salman nothing**, because salman has
no convention to disagree with. The vendor-specific part of Modbus addressing is the
mapping from the four data tables onto a device's application, which APS §4.4 says is
"totally vendor device specific". salman's IO mapping layer *is* that vendor-specific
part, and it says so rather than pretending to be a standard.

**Word order across registers is a separate decision and is not made here.** The
specification defines byte order within a register and says nothing about the order of
registers within a 32-bit value. salman therefore requires the word order to be stated
explicitly wherever a value spans registers, with no default. Refusing to guess is the
correct behaviour, not a gap.
