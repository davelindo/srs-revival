"""Build the verified two-instruction X7 shell patch locally; never flash it."""

import argparse
import hashlib
import json
from pathlib import Path
import struct

import bcd

STOCK_SHA256 = "2f8af043a5058bc368ec99222ca391770ac75aca486719281e71cef03d12e129"
PATCHED_SHA256 = "9afe62ce41d6d2b5d1834d5adcffd46552a6841f25165e3f04d532de751804d2"
BASE = 0x60008000
PATCHES = (
    (0x60266D9C, "0250a0e1", "915ea0e3"),
    (0x60266DA0, "0140a0e1", "0140a0e3"),
)


def build(stock):
    if hashlib.sha256(stock).hexdigest() != STOCK_SHA256:
        raise ValueError("requires the exact original PSK004 Laphroaigv2.09a.bcd")
    sections = bcd.sections(stock)
    bcd.verify_boot(sections[0]["data"])
    app_offset = sections[0]["off"]
    app_size = sections[0]["size"]
    output = bytearray(stock)
    allowed = set(range(0x2C, 0x30)) | set(range(0x3C, 0x40))
    allowed |= set(range(app_offset + 0x14, app_offset + 0x18))
    allowed |= set(range(app_offset + 0x2C, app_offset + 0x30))
    for address, before, after in PATCHES:
        offset = app_offset + bcd.STAGE2_OFF + address - BASE
        if output[offset:offset + 4] != bytes.fromhex(before):
            raise ValueError(f"unexpected instruction at {address:#x}")
        output[offset:offset + 4] = bytes.fromhex(after)
        allowed.update(range(offset, offset + 4))
    app = bytearray(output[app_offset:app_offset + app_size])
    struct.pack_into("<I", app, 0x2C, bcd.fletcher_dm870(app[bcd.STAGE2_OFF:]))
    words = struct.unpack_from("<I", app, 0x10)[0]
    struct.pack_into("<I", app, 0x14, bcd.crc32(app[0x18:0x18 + words * 8]))
    output[app_offset:app_offset + app_size] = app
    struct.pack_into("<I", output, 0x3C, bcd.crc32(app))
    struct.pack_into("<I", output, 0x2C, bcd.header_crc(output))
    final = bytes(output)
    checked = bcd.sections(final)
    bcd.verify_boot(checked[0]["data"])
    changed = {index for index, (a, b) in enumerate(zip(stock, final)) if a != b}
    if len(final) != len(stock) or not changed <= allowed:
        raise ValueError("unexpected bytes changed")
    if any(a["data"] != b["data"] for a, b in zip(sections[1:], checked[1:])):
        raise ValueError("configuration or resources changed")
    if hashlib.sha256(final).hexdigest() != PATCHED_SHA256:
        raise ValueError("output differs from the image verified on hardware")
    return final, len(changed)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("stock", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    final, changed = build(args.stock.read_bytes())
    with args.output.open("xb") as target:
        target.write(final)
    print(json.dumps({"output": str(args.output), "bytes": len(final),
                      "sha256": hashlib.sha256(final).hexdigest(),
                      "changed_bytes_including_checksums": changed,
                      "console_port": 2320}, indent=2))


if __name__ == "__main__":
    main()
