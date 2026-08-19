# salman-cli

The `salman` command line interface — the whole product in one binary.

```bash
cargo install salman-cli
```

Installs a single self-contained executable called `salman`. There is no
installer, no service and no registry key, and deleting the file uninstalls it.

```
salman check <file.st>            parse and type-check
salman run   <file.st>            compile and run on the simulation runtime
salman test  <file.st> <tests/>   run declarative tests, headless
salman capture <file.pcap>        say what happened on a packet capture
salman project <file.yaml>        check a project file
salman status                     what salman can do, and how far it is tested
```

`salman test` exits non-zero on failure and can write JUnit XML, so the same
command is a CI job. There is no GUI to drive and no licence to check out.

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
