# salman-findings

The claims salman makes about decoded bytes, and the evidence for each.

A compiler knows its whole input and is entitled to be certain. A capture is a
partial view of something that already happened, recorded by a tool that made its
own decisions about what to keep. So everything here is built around saying **how
sure salman is and why**, and around being able to say "I could not tell" as a
first-class answer rather than as silence.

A finding has three independent axes: what kind of claim it is, how bad it is,
and what sort of thing was observed. Collapsing them is how a tool ends up unable
to express "I checked this and it was fine" — which is the answer that makes the
other answers trustworthy.

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
