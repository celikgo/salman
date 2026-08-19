# salman-link

Running a project's IO mappings against a device, at the scan boundaries.

A scan is: latch the inputs, run the program, publish the outputs. A mapping
hooks either side of that — inputs are read from the device *before* the latch,
outputs written *after* the publish — so the program sees one frozen picture of
the world for the whole scan, which is the property the process image exists to
give.

**salman does not drive a plant, and this is where that is enforced.** An
engineering write is one value, to one register, once, because a person decided
to. A control loop writes its outputs every scan, for ever; confirming each one
is not possible, and a tool that asked once and then wrote ten thousand times
would have turned a per-call confirmation into a session-wide licence to drive a
plant. So salman does not do it: **output mappings run against a simulated device
only.** Against a live device a link may read, and salman refuses to write.

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
