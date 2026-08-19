# salman-modbus

Modbus protocol data units, framing and checksums, independent of any transport.

This crate is **pure**: it opens no socket, reads no file and starts no thread.
Bytes go in and typed frames come out, which is what makes the same decoder
usable on a live socket and on a capture file. A decoder that could only be
exercised against real equipment could not be tested at all.

Every constant is transcribed from a document salman fetched and can cite — the
MODBUS Application Protocol Specification V1.1b3, the Serial Line guide V1.02
and the Messaging on TCP/IP guide V1.0b. No specification text is reproduced.
Where those documents are silent or disagree with themselves, salman makes a
decision, marks it as salman's, and never presents it as the specification's.

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
