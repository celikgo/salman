# salman-capture

Packet captures: reading them, writing them, and decoding what is inside.

salman reads a capture the same way it reads a live socket, and the decoder above
this layer cannot tell which it is. That is why every protocol decoder in this
workspace takes bytes rather than a connection.

Classic pcap is written in-crate rather than taken as a dependency, which buys
three things: `unsafe_code = "forbid"` provable across the whole path from file
to decoded frame, a fuzz target salman owns so that every finding is actionable,
and errors in salman's own diagnostic vocabulary from the first line rather than
translated from a foreign enum.

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
