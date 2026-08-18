#!/usr/bin/env python3
"""pymodbus, as an independent client, driving salman's simulator.

Exits non-zero if anything salman answered disagrees with what the simulator
was seeded with. The point is not that salman agrees with itself — the test
suite covers that — but that an implementation nobody here wrote reads the same
bytes the same way.
"""
import sys

from pymodbus.client import ModbusTcpClient

FAILURES = []


def check(what, got, expected):
    if list(got) != list(expected):
        FAILURES.append(f"{what}: pymodbus read {list(got)}, salman was seeded with {list(expected)}")
    else:
        print(f"ok   {what}: {list(got)}")


def main():
    port = int(sys.argv[1])
    client = ModbusTcpClient("127.0.0.1", port=port)
    if not client.connect():
        print(f"could not connect to 127.0.0.1:{port}", file=sys.stderr)
        return 2

    # Reads, against what examples/simulator.rs seeds.
    r = client.read_holding_registers(address=0, count=5, device_id=1)
    check("holding registers", r.registers, [0x1000 + i for i in range(5)])
    r = client.read_input_registers(address=0, count=5, device_id=1)
    check("input registers", r.registers, [0x2000 + i for i in range(5)])
    r = client.read_discrete_inputs(address=0, count=9, device_id=1)
    check("discrete inputs", [int(b) for b in r.bits[:9]], [1 if i % 3 == 0 else 0 for i in range(9)])

    # Writes, read back through salman.
    client.write_register(address=3, value=0xCAFE, device_id=1)
    client.write_registers(address=5, values=[11, 22, 33], device_id=1)
    client.write_coil(address=1, value=True, device_id=1)
    client.write_coils(address=4, values=[True, False, True, True], device_id=1)

    r = client.read_holding_registers(address=3, count=1, device_id=1)
    check("single register written", r.registers, [0xCAFE])
    r = client.read_holding_registers(address=5, count=3, device_id=1)
    check("multiple registers written", r.registers, [11, 22, 33])
    r = client.read_coils(address=0, count=8, device_id=1)
    check("coils written", [int(b) for b in r.bits[:8]], [0, 1, 0, 0, 1, 0, 1, 1])

    # An address the simulator does not have. A conforming server answers
    # exception 02, and pymodbus must see it as an error rather than a value.
    r = client.read_holding_registers(address=60000, count=4, device_id=1)
    if not r.isError():
        FAILURES.append(f"out of range: expected an exception, got {r}")
    elif getattr(r, "exception_code", None) != 2:
        FAILURES.append(f"out of range: expected exception 2, got {r}")
    else:
        print("ok   out of range: exception 2, Illegal Data Address")

    client.close()

    for failure in FAILURES:
        print(f"FAIL {failure}", file=sys.stderr)
    return 1 if FAILURES else 0


if __name__ == "__main__":
    sys.exit(main())
