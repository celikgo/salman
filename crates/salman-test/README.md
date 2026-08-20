# salman-test

Unit and golden-trace tests for IEC 61131-3 code.

This is the part of salman that makes PLC code testable the way software is
testable: a declarative test runs a program on salman's own runtime, on any
operating system, in a container, with no vendor licence and no Windows virtual
machine.

PLC unit testing is not new. Every open-source framework salman's authors could
find requires a proprietary runtime — TwinCAT, CODESYS, Sysmac Studio or TIA
Portal. What is absent, and what this crate is for, is doing it without one.

A green suite here says the code does what the test says, on salman's runtime,
under a virtual clock. It is not evidence for a functional safety argument.

## Part of salman

[salman](https://github.com/celikgo/salman) is a vendor-neutral, text-first,
git-native workbench for IEC 61131-3 PLC engineering. Structured Text compiles
and runs on a deterministic runtime, and its tests run headless in CI with no
vendor licence.

**Version 0.1.0, pre-alpha.** The interface will change.
[`docs/CONFORMANCE.md`](https://github.com/celikgo/salman/blob/main/docs/CONFORMANCE.md)
states exactly what is implemented and what is tested.

## Not a safety tool

salman is an engineering and diagnostic tool. It is **not** a safety PLC, is not
certified under IEC 61508, IEC 62061 or ISO 13849, and must never be used to
design, validate or replace a safety function. See
[`LEGAL.md`](https://github.com/celikgo/salman/blob/main/LEGAL.md).

## Licence

Apache-2.0.
