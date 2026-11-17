"""Validate/decode the observed AU002 ENC0003/HAS0003 protocol offline."""

import argparse
import hashlib
import hmac
import json
from pathlib import Path
import xml.etree.ElementTree as ET

from Crypto.Cipher import AES

KEY = bytes.fromhex("0123456789abcdeffedcba9876543210")
CATEGORY = "AU002"
SERVICE = "PSK004"


def decode(blob, category=CATEGORY, service=SERVICE):
    header, separator, ciphertext = blob.partition(b"\n\n")
    if not separator:
        raise ValueError("missing manifest header separator")
    fields = dict(line.split(b":", 1) for line in header.splitlines())
    if fields.get(b"eaid") != b"ENC0003" or fields.get(b"daid") != b"HAS0003":
        raise ValueError("expected ENC0003/HAS0003")
    if not ciphertext or len(ciphertext) % 16:
        raise ValueError("incomplete AES blocks")
    plaintext = AES.new(KEY, AES.MODE_ECB).decrypt(ciphertext)
    inner = hashlib.sha1(plaintext).hexdigest()
    digest = hashlib.sha1((inner + service + category).encode("ascii")).hexdigest()
    if not hmac.compare_digest(fields.get(b"digest", b""), digest.encode("ascii")):
        raise ValueError("manifest digest mismatch")
    root = ET.fromstring(plaintext)
    if root.tag != "InformationFile":
        raise ValueError("unexpected XML root")
    return plaintext, root


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    args = parser.parse_args()
    plaintext, root = decode(args.manifest.read_bytes())
    print(json.dumps({
        "category": CATEGORY, "service": SERVICE, "digest_matches": True,
        "padded_plaintext_bytes": len(plaintext),
        "distributions": [node.attrib for node in root.iter("Distribution")],
    }, indent=2))


if __name__ == "__main__":
    main()
