"""Read/check the three-section BridgeCo container used by the SRS-X7.

Adapted from zd1 tools/zeppelin/fw/bcd.py, commit
1cd511c0d37a758d6267675664bf78abd36a8ee6.
Copyright (c) 2026 Alex Pawlowski. MIT; see LICENSES/zd1.txt.
"""

import struct
import zlib

HDR_LEN = 0xB0
STAGE2_OFF = 0x2000


def crc32(data):
    return zlib.crc32(data) & 0xFFFFFFFF


def fletcher_dm870(data):
    """16-bit LE words; 0xffff accumulators, folded every 360 words."""
    if len(data) & 1:
        data += b"\0"
    a = b = 0xFFFF
    words = struct.unpack_from(f"<{len(data) // 2}H", data)
    for start in range(0, len(words), 360):
        for word in words[start:start + 360]:
            a += word
            b += a
        a = (a & 0xFFFF) + (a >> 16)
        b = (b & 0xFFFF) + (b >> 16)
    a = (a & 0xFFFF) + (a >> 16)
    b = (b & 0xFFFF) + (b >> 16)
    return (a | (b << 16)) & 0xFFFFFFFF


def header_crc(blob):
    header = bytearray(blob[:HDR_LEN])
    header[0x2C:0x30] = bytes(4)
    return crc32(header)


def sections(blob):
    if len(blob) < HDR_LEN + 8 or blob[:4] != b"bCoD":
        raise ValueError("expected a complete bCoD header")
    if struct.unpack_from("<I", blob, 0x2C)[0] != header_crc(blob):
        raise ValueError("container header CRC mismatch")
    result = []
    end = HDR_LEN + 8
    for index in range(3):
        offset, nand, size, crc, capacity = struct.unpack_from(
            "<5I", blob, 0x30 + 0x20 * index
        )
        if not size or offset < end or offset + size > len(blob):
            raise ValueError(f"invalid or overlapping section {index}")
        data = blob[offset:offset + size]
        if crc32(data) != crc:
            raise ValueError(f"section {index} CRC mismatch")
        result.append(dict(idx=index, off=offset, nand=nand, size=size,
                           psize=capacity, data=data))
        end = offset + size
    return result


def verify_boot(app):
    if len(app) < STAGE2_OFF or app[:4] != b"\x55\x55\xaa\xaa":
        raise ValueError("missing application boot header")
    values = struct.unpack_from("<12I", app)
    stage1_end = 0x18 + values[4] * 8
    if stage1_end > STAGE2_OFF:
        raise ValueError("stage-1 length exceeds preloader")
    if values[10] != len(app) - STAGE2_OFF:
        raise ValueError("stage-2 length mismatch")
    if values[11] != fletcher_dm870(app[STAGE2_OFF:]):
        raise ValueError("stage-2 Fletcher mismatch")
    if values[5] != crc32(app[0x18:stage1_end]):
        raise ValueError("stage-1 CRC mismatch")
