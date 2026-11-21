"""Verify the owner credentials and nested archives of an assembled SRS-X7 bundle."""

import argparse
import gzip
import hashlib
from pathlib import Path
import stat
import struct
import zlib


def require(condition, message):
    if not condition:
        raise ValueError(message)


def members(compressed):
    data = gzip.decompress(compressed)
    entries = {}
    offset = 0
    while data[offset:offset + 6] == b"070701":
        start = offset + 6
        header = [int(data[start + n:start + n + 8], 16) for n in range(0, 104, 8)]
        name_at = start + 104
        name_end = name_at + header[11]
        require(header[11] and data[name_end - 1:name_end] == b"\0", "invalid archive name")
        name = data[name_at:name_end - 1].decode("utf-8")
        body = (name_end + 3) & ~3
        end = body + header[6]
        require(end <= len(data), "truncated archive entry")
        if name == "TRAILER!!!":
            require(not any(data[end:]), "unexpected archive trailer")
            return entries
        require(name not in entries, "duplicate archive entry")
        entries[name] = (header[1], data[body:end])
        offset = (end + 3) & ~3
    raise ValueError("missing newc archive trailer")


def verify(bundle, owner):
    system = (bundle / "system.cpio.gz").read_bytes()
    rescue = (bundle / "recovery.cpio.gz").read_bytes()
    image = (bundle / "sony-x7.bcd").read_bytes()
    checks = dict(line.split("  ", 1)[::-1] for line in (bundle / "SHA256SUMS").read_text().splitlines())
    for name, data in (("sony-x7.bcd", image), ("system.cpio.gz", system), ("recovery.cpio.gz", rescue)):
        require(hashlib.sha256(data).hexdigest() == checks[name], f"{name} digest mismatch")
    slot_id = (hashlib.sha256(system).hexdigest() + "\n").encode()
    require((bundle / "system-id").read_bytes() == slot_id, "slot digest mismatch")
    app, _, app_size = struct.unpack_from("<3I", image, 0x30)
    payload = image[app + 0x2000:app + app_size]
    require(payload[:(bundle / "payload.bin").stat().st_size] == (bundle / "payload.bin").read_bytes(),
            "BCD application differs from the assembled payload")
    require(payload[0x40:0x44] == b"ZLB1", "unexpected boot payload header")
    kernel_at, kernel_len, initrd_at, initrd_len = struct.unpack_from("<4I", payload, 0x50)
    kernel = zlib.decompress(payload[kernel_at:kernel_at + kernel_len], -15)
    require(kernel == (bundle / "kernel-rx.bin").read_bytes(), "boot kernel mismatch")
    require(payload[initrd_at:initrd_at + initrd_len] == rescue, "boot payload contains a different rescue")

    slot, recovery = members(system), members(rescue)
    require(recovery["usr/share/x7/system.cpio.gz"][1] == system, "rescue contains a different system")
    require(recovery["usr/share/x7/system-id"][1] == slot_id, "rescue contains a different slot digest")
    prefix = "usr/share/x7/provisioning/"
    for archive, is_slot in ((slot, True), (recovery, False)):
        names = {"authorized_keys", "web-password"}
        if is_slot:
            names.update(("CountryCode", "NetIFMode"))
        require({n.removeprefix(prefix) for n in archive if n.startswith(prefix)} == names,
                "unexpected provisioning file list")
        for name in names:
            mode, value = archive[prefix + name]
            require(stat.S_ISREG(mode) and value == (owner / name).read_bytes(), f"owner {name} mismatch")
            if name in ("authorized_keys", "web-password"):
                require(stat.S_IMODE(mode) == 0o600, f"unexpected permissions for {name}")
    print("Verified BCD payload, kernel, both archives, slot digest and owner provisioning.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("bundle", type=Path)
    parser.add_argument("owner", type=Path)
    args = parser.parse_args()
    verify(args.bundle, args.owner)
