"""Decode X7 SPI frames or summarize saved spilog text; no hardware transport."""

import argparse
from collections import Counter
import json
from pathlib import Path
import re
import struct


def crc8(data):
    value = 0
    for byte in data:
        value ^= byte
        for _ in range(8):
            value = ((value << 1) ^ (0x85 if value & 0x80 else 0)) & 0xFF
    return value


def unstuff(wire):
    result = bytearray()
    escaped = False
    for byte in wire:
        if byte == 0xFF:
            continue
        if escaped:
            if byte not in (0x01, 0x27):
                raise ValueError("invalid escape sequence")
            result.append(0xFF if byte == 0x01 else 0x27)
            escaped = False
        elif byte == 0x27:
            escaped = True
        else:
            result.append(byte)
    if escaped:
        raise ValueError("truncated escape sequence")
    return bytes(result)


def decode_frame(wire, direction):
    if direction not in ("request", "reply"):
        raise ValueError("direction must be request or reply")
    frame = unstuff(wire)
    if len(frame) < 5 or crc8(frame[:4]) != frame[4]:
        raise ValueError("incomplete header or header CRC mismatch")
    kind, register, length = struct.unpack_from("<BBH", frame)
    if kind not in (0, 1, 2, 3):
        raise ValueError("unsupported operation; normal read/write frames only")
    if kind in (0, 1) and length:
        raise ValueError("fixed-operation reserved bytes must be zero")
    has_body = kind in ((0, 2) if direction == "request" else (1, 3))
    count = (2 if kind in (0, 1) else length) if has_body else 0
    if len(frame) != 5 + count + int(has_body):
        raise ValueError("frame length does not match operation/direction")
    body = frame[5:5 + count]
    if has_body and crc8(body) != frame[-1]:
        raise ValueError("body CRC mismatch")
    return {"direction": direction, "kind": kind, "register": f"0x{register:02x}",
            "length_field": length, "body_hex": body.hex(), "crc_valid": True}


def summarize(text):
    counts = Counter()
    total = 0
    for match in re.finditer(r"(?m)^\s*(\d+)\s+(\d+)\s+(\d+)\s+(\d+)\s+(\d+)\s+(\d+)\s*$", text):
        timestamp, marker, kind, register, byte2, byte3 = map(int, match.groups())
        if timestamp > 65535 or any(v > 255 for v in (marker, kind, register, byte2, byte3)):
            raise ValueError("out-of-range spilog field")
        counts[kind, register] += 1
        total += 1
    if not total:
        raise ValueError("no six-column decimal spilog records found")
    return {"records": total, "scope": "Historical decoded headers; no bodies or raw wire bytes",
            "counts": [{"kind": kind, "register": f"0x{register:02x}", "count": count}
                       for (kind, register), count in counts.most_common()]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    decoder = commands.add_parser("decode")
    decoder.add_argument("--direction", choices=("request", "reply"), required=True)
    decoder.add_argument("--hex", required=True)
    log = commands.add_parser("summarize")
    log.add_argument("input", type=Path)
    args = parser.parse_args()
    if args.command == "decode":
        result = decode_frame(bytes.fromhex(args.hex), args.direction)
    else:
        result = summarize(args.input.read_text(encoding="utf-8"))
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
