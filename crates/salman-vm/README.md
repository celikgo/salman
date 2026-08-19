# salman-vm

A bytecode compiler and a deterministic scan runtime for IEC 61131-3.

Not an AST interpreter, because walking a tree per scan makes scan cost depend
on source shape in ways that are hard to budget. Not a transpiler, because
compiling to another language puts that language's arithmetic and its optimiser
between salman and the determinism promise. A bytecode VM is the only one of the
three where salman decides, and can state, exactly what every operation does.

Single-threaded by design: floating-point addition is not associative, so any
parallel reduction over reals reassociates with thread scheduling and cannot
produce a reproducible answer. Run the same program twice, on two machines, and
the recorded trace fingerprint is identical.

This runtime is for development, testing and simulation. It is not certified and
is not for controlling machinery.

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
