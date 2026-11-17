"""Extract the X7's register metadata into a private local JSON catalogue."""

import argparse
import hashlib
import json
from pathlib import Path
import struct

import bcd
from build_telnet import BASE, STOCK_SHA256


def catalogue(stock):
    if hashlib.sha256(stock).hexdigest() != STOCK_SHA256:
        raise ValueError("requires the pinned original X7 Laphroaig image")
    app = bcd.sections(stock)[0]["data"]
    bcd.verify_boot(app)
    image = app[bcd.STAGE2_OFF:]
    records = []
    for address in range(0x6051C9A4, 0x6051D3A4, 20):
        pointer, register, capacity, word3, word4 = struct.unpack_from(
            "<5I", image, address - BASE
        )
        offset = pointer - BASE
        if not 0 <= offset < len(image) or not 0 <= register <= 255:
            raise ValueError("invalid register descriptor")
        end = image.find(b"\0", offset, offset + 128)
        if end < 0:
            raise ValueError("unterminated register name")
        records.append({"register": f"0x{register:02x}",
                        "name": image[offset:end].decode("ascii"),
                        "capacity": capacity, "table_address": f"0x{address:08x}",
                        "word3": word3, "word4": word4})
    return records


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("stock", type=Path)
    parser.add_argument("--output", type=Path, default=Path("work/registers.json"))
    args = parser.parse_args()
    records = catalogue(args.stock.read_bytes())
    with args.output.open("x", encoding="utf-8") as output:
        json.dump(records, output, indent=2)
        output.write("\n")
    print(f"Wrote {len(records)} static descriptors to {args.output}; no live values read")


if __name__ == "__main__":
    main()
