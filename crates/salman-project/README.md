# salman-project

The salman project file: sources, devices, and how their registers reach the
process image.

A project says three things: which source files make up the program, which
devices it talks to, and how those devices' registers reach the process image.
The third is the one with no good home anywhere else — putting an IO mapping in
code means it cannot be reviewed by the person who knows the plant, and putting
it in a vendor tool means it cannot be reviewed at all.

```yaml
dialect: generic
sources:
  - conveyor.st
devices:
  - name: press
    protocol: modbus-tcp
    address: "10.4.2.7:502"
    unit: 1
    map:
      - table: input-registers
        from: 0
        count: 4
        to: "%IW0"
```

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
