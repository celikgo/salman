# salman-analyse

Reading a capture and saying what happened on it.

The layers below this one produce **facts**: these bytes were at this offset,
this stream carried these bytes, this frame decoded to this request. This layer
produces **claims**, and every claim points back at the facts that support it.

The decode path is heavily fuzzed and makes no judgements; the analysis makes
judgements and can be wrong without the decoders being wrong. It also means a
finding salman got wrong is one somebody can argue with, because the evidence is
attached to it.

It does not try to decide whether a plant is healthy. An exception is worth
surfacing and an unanswered request is worth surfacing; salman stops well short
of guessing, because a hundred low-precision findings is how a diagnostic tool
loses its reader.

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
