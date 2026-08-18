#!/usr/bin/env python3
"""pymodbus, as an independent server, for salman's client to drive.

Seeded so that every table holds something different: a client reading the
wrong one is then obvious rather than plausible.
"""
import sys
import warnings

warnings.filterwarnings("ignore")

from pymodbus.datastore import (
    ModbusDeviceContext,
    ModbusSequentialDataBlock,
    ModbusServerContext,
)
from pymodbus.server import StartTcpServer


def main():
    port = int(sys.argv[1])
    # pymodbus's block refuses a starting address of zero — its own historical
    # off-by-one — so the blocks start at 1 and its context subtracts one
    # internally. salman applies no offset anywhere; see
    # docs/adr/ADR-0012-modbus-addressing.md. That the two agree on what PDU
    # address 0 means is part of what this harness checks.
    store = ModbusDeviceContext(
        di=ModbusSequentialDataBlock(1, [1, 0, 1, 1, 0, 0, 1, 0] * 16),
        co=ModbusSequentialDataBlock(1, [0, 1, 1, 0, 1, 0, 0, 1] * 16),
        hr=ModbusSequentialDataBlock(1, [0x1000 + i for i in range(200)]),
        ir=ModbusSequentialDataBlock(1, [0x2000 + i for i in range(200)]),
    )
    context = ModbusServerContext(devices=store, single=True)
    print("ready", flush=True)
    StartTcpServer(context=context, address=("127.0.0.1", port))


if __name__ == "__main__":
    main()
