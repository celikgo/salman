# salman-plcopen

Reading and writing the PLCopen XML exchange format.

Targets **v2.01** — "XML Formats for IEC 61131-3", Official Release 2009-05-08,
namespace `http://www.plcopen.org/xml/tc6_0201`. There is no v3.0, and IEC
61131-10 is a different, incompatible format rather than a later one.

**The thing that surprises everyone:** Structured Text is not stored as text.
`<ST>` has the schema type `formattedText`, whose entire definition is a sequence
of exactly one element from the XHTML namespace. So `<ST>a := TRUE;</ST>` does
not validate, and neither does a bare CDATA section — the code has to sit inside
an XHTML element, and the specification never says which one.

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
