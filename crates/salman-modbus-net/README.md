# salman-modbus-net

Modbus TCP over real sockets: a client, and a simulator to point it at.

`salman-modbus` decides what frames mean and this crate carries them. The split
is what makes the protocol testable without hardware.

**The posture model is not optional here.** This is the first code path in salman
that can change a real device:

- A **read** needs no permission.
- A **write to a real device** needs the ARMED posture *and* a human's
  confirmation of that specific call. The confirmation is taken **by value**, so
  one confirmation authorises exactly one write and cannot be kept and reused.
  The type has no public constructor, so no caller — agent or otherwise — can
  manufacture one.
- Running the **simulator** needs the SIMULATE posture.

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
