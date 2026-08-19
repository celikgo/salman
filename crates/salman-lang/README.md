# salman-lang

The IEC 61131-3 front end: lexer, parser, AST and type checker.

Targets **IEC 61131-3 Edition 3.0 (2013)**. Where the code cites a clause or
table number it means that edition, and the citation registry records how far
each number could be verified against a public source.

**What is implemented:** Structured Text. The graphical languages (LD, FBD,
SFC), Instruction List and the Edition 3 object-oriented extensions are not —
their keywords are reserved so that meeting one produces a clear message rather
than a baffling syntax error.

Every input is treated as hostile: nesting, source size and identifier length
are all bounded, no path may panic on malformed input, and the parsers are
fuzzed in CI.

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
