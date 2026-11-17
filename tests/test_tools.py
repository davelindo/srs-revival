"""Synthetic integrity/protocol fixtures; no vendor image or hardware needed."""

import hashlib
from pathlib import Path
import struct
import sys
import unittest

from Crypto.Cipher import AES

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))
import bcd
import build_telnet
import manifest
import spi


def synthetic_manifest(plaintext, service="PSK004"):
    padded = plaintext + b" " * (-len(plaintext) % 16)
    digest = hashlib.sha1((hashlib.sha1(padded).hexdigest() + service + "AU002").encode()).hexdigest()
    return (f"eaid:ENC0003\ndaid:HAS0003\ndigest:{digest}\n\n".encode()
            + AES.new(manifest.KEY, AES.MODE_ECB).encrypt(padded))


def synthetic_bcd():
    app = bytearray(bcd.STAGE2_OFF + 6)
    app[:4] = b"\x55\x55\xaa\xaa"
    struct.pack_into("<I", app, 0x10, 12)
    app[bcd.STAGE2_OFF:] = bytes.fromhex("010002000300")
    struct.pack_into("<I", app, 0x28, 6)
    struct.pack_into("<I", app, 0x2C, 0x000A0006)
    struct.pack_into("<I", app, 0x14, bcd.crc32(app[0x18:0x78]))
    data = [bytes(app), b"synthetic config", b"synthetic resource"]
    header = bytearray(bcd.HDR_LEN + 8)
    header[:4] = b"bCoD"
    offset = len(header)
    for index, section in enumerate(data):
        struct.pack_into("<5I", header, 0x30 + index * 0x20,
                         offset, 0, len(section), bcd.crc32(section), len(section))
        offset += len(section)
    struct.pack_into("<I", header, 0x2C, bcd.header_crc(header))
    return bytes(header) + b"".join(data)


class ManifestTests(unittest.TestCase):
    def test_padding_remains_in_validated_plaintext(self):
        xml = b'<InformationFile><Distribution ID="test"/></InformationFile>'
        plaintext, root = manifest.decode(synthetic_manifest(xml))
        self.assertTrue(plaintext.endswith(b" "))
        self.assertEqual(len(plaintext) % 16, 0)
        self.assertEqual(root.find("Distribution").get("ID"), "test")

    def test_integrity_binds_service_identity(self):
        blob = synthetic_manifest(b"<InformationFile/>")
        with self.assertRaisesRegex(ValueError, "digest"):
            manifest.decode(blob, service="PSK003")

    def test_ciphertext_corruption_is_rejected(self):
        blob = bytearray(synthetic_manifest(b"<InformationFile/>"))
        blob[-1] ^= 1
        with self.assertRaisesRegex(ValueError, "digest"):
            manifest.decode(bytes(blob))

    def test_partial_aes_block_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "AES"):
            manifest.decode(synthetic_manifest(b"<InformationFile/>")[:-1])


class ContainerTests(unittest.TestCase):
    def test_known_checksums_and_nested_boot(self):
        self.assertEqual(bcd.crc32(b"123456789"), 0xCBF43926)
        self.assertEqual(bcd.fletcher_dm870(bytes.fromhex("010002000300")), 0x000A0006)
        sections = bcd.sections(synthetic_bcd())
        bcd.verify_boot(sections[0]["data"])

    def test_section_corruption_is_rejected(self):
        blob = bytearray(synthetic_bcd())
        blob[-1] ^= 1
        with self.assertRaisesRegex(ValueError, "section 2 CRC"):
            bcd.sections(bytes(blob))

    def test_header_corruption_is_rejected(self):
        blob = bytearray(synthetic_bcd())
        blob[0x18] ^= 1
        with self.assertRaisesRegex(ValueError, "header CRC"):
            bcd.sections(bytes(blob))

    def test_truncation_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "section 2"):
            bcd.sections(synthetic_bcd()[:-1])

    def test_patcher_refuses_unrecognized_input(self):
        with self.assertRaisesRegex(ValueError, "exact original"):
            build_telnet.build(synthetic_bcd())


class SpiTests(unittest.TestCase):
    def test_crc_check_vector(self):
        self.assertEqual(spi.crc8(b"123456789"), 0x2A)

    def test_fixed_read_request(self):
        result = spi.decode_frame(bytes.fromhex("011100007f"), "request")
        self.assertEqual(result["register"], "0x11")
        self.assertEqual(result["body_hex"], "")

    def test_reply_requires_body(self):
        with self.assertRaisesRegex(ValueError, "length"):
            spi.decode_frame(bytes.fromhex("011100007f"), "reply")

    def test_escaping_and_idle(self):
        self.assertEqual(spi.unstuff(bytes.fromhex("ff27012727ff")), b"\xff\x27")
        with self.assertRaisesRegex(ValueError, "truncated"):
            spi.unstuff(b"\x27")
        with self.assertRaisesRegex(ValueError, "invalid escape"):
            spi.unstuff(b"\x27\x02")

    def test_header_corruption(self):
        with self.assertRaisesRegex(ValueError, "header CRC"):
            spi.decode_frame(bytes.fromhex("011000007f"), "request")

    def test_body_integrity(self):
        # Synthetic variable-write frame, deliberately damaged after its CRC is set.
        header = bytes.fromhex("027e0300")
        body = b"ABC"
        plain = header + bytes([spi.crc8(header)]) + body + bytes([spi.crc8(body)])
        self.assertEqual(spi.decode_frame(plain, "request")["body_hex"], body.hex())
        damaged = bytearray(plain)
        damaged[5] ^= 1
        with self.assertRaisesRegex(ValueError, "body CRC"):
            spi.decode_frame(bytes(damaged), "request")

    def test_log_summary_does_not_invent_payloads(self):
        result = spi.summarize("time mark kind reg b2 b3\n100 0 1 0 0 0\n101 0 1 0 0 0\n102 0 2 64 4 0\n")
        self.assertEqual(result["records"], 3)
        self.assertEqual(result["counts"][0], {"kind": 1, "register": "0x00", "count": 2})
        with self.assertRaisesRegex(ValueError, "no six-column"):
            spi.summarize("not a capture")


if __name__ == "__main__":
    unittest.main()
