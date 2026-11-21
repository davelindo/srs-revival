#!/usr/bin/env python3
"""Build and insert the receive-ring fix into the exact Nokia Kernel_A Image."""

import argparse
import hashlib
import struct
import subprocess
import tempfile
from pathlib import Path

IMAGE_BASE = 0xC0008000
IMAGE_SHA = "66763b0011bed2c9e21282d5e8ed24e1c048d3b13aa062b76a9dfe854e3d2eff"
TEXT_PAD = 0xC02B1D64  # __end_rodata to _etext, after the last kernel parameter.
TEXT_END = 0xC02B2000
SOURCE = Path(__file__).resolve().parent


def address_offset(addr):
    return addr - IMAGE_BASE


def branch(addr, target):
    displacement = target - (addr + 8)
    if displacement % 4 or not -(1 << 25) <= displacement < (1 << 25):
        raise ValueError("ARM branch out of range")
    return 0xEB000000 | ((displacement // 4) & 0xFFFFFF)


def replace(image, addr, expected, new):
    offset = address_offset(addr)
    if struct.unpack_from("<I", image, offset)[0] != expected:
        raise ValueError(f"Unexpected instruction or literal at {addr:08x}")
    struct.pack_into("<I", image, offset, new)


def build(args, work):
    cc = [
        args.clang,
        "--target=armv5te-none-eabi",
        "-marm",
        "-Os",
        "-ffreestanding",
        "-fno-unwind-tables",
        "-fno-asynchronous-unwind-tables",
        "-fno-stack-protector",
        "-Wall",
        "-Wextra",
        "-Werror",
    ]
    impl, hooks, elf, code = (work / name for name in ("rx.o", "hooks.o", "rx.elf", "rx.bin"))
    subprocess.run(cc + ["-c", SOURCE / "ethernet-rx.c", "-o", impl], check=True)
    subprocess.run(cc + ["-c", SOURCE / "ethernet-rx-hooks.S", "-o", hooks], check=True)
    subprocess.run(
        [
            args.linker,
            f"-Ttext={TEXT_PAD:#x}",
            "-e",
            "x7_eth_read_isr",
            "-o",
            elf,
            hooks,
            impl,
        ],
        check=True,
    )
    subprocess.run([args.objcopy, "-O", "binary", "--only-section=.text", elf, code], check=True)
    lines = subprocess.check_output([args.nm, "--defined-only", elf], text=True).splitlines()
    symbols = {name: int(addr, 16) for addr, _, name in (line.split() for line in lines)}
    return code.read_bytes(), symbols


def replace_direct_kernel(payload, original, patched):
    kernel, size, initrd, initrd_size, tags_size = struct.unpack_from("<5I", payload, 0x400)
    if (
        kernel != 0x1000
        or size != len(original)
        or initrd != kernel + size
        or initrd + initrd_size > len(payload)
        or tags_size > kernel - 0x440
        or payload[kernel:initrd] != original
    ):
        raise ValueError("Direct boot payload does not contain the expected raw kernel")
    return payload[:kernel] + patched + payload[initrd:]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("image", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--direct-boot", type=Path, help="replace the kernel in a direct boot payload")
    parser.add_argument("--clang", default="clang")
    parser.add_argument("--linker", default="ld.lld")
    parser.add_argument("--objcopy", default="llvm-objcopy")
    parser.add_argument("--nm", default="llvm-nm")
    args = parser.parse_args()
    original = args.image.read_bytes()
    if hashlib.sha256(original).hexdigest() != IMAGE_SHA:
        parser.error("unsupported kernel Image")

    with tempfile.TemporaryDirectory() as directory:
        code, symbols = build(args, Path(directory))
    start, end = map(address_offset, (TEXT_PAD, TEXT_END))
    if not code or len(code) > end - start or any(original[start:end]):
        raise ValueError("Patch exceeds the verified zero-filled text padding")
    isr, dma = (symbols[name] for name in ("x7_eth_read_isr", "x7_eth_rx_dma"))
    if not all(TEXT_PAD <= x < TEXT_PAD + len(code) for x in (isr, dma)):
        raise ValueError("Hook entries outside the patch")

    image = bytearray(original)
    replace(image, 0xC014B438, 0xE3A03082, 0xE3A03086)  # Enable buffer-unavailable IRQ.
    replace(image, 0xC014C38C, 0xE3A02000, 0xE3A02020)  # IRQF_DISABLED on the MAC handler.
    replace(image, 0xC014C464, 0xE5933024, branch(0xC014C464, isr))
    replace(image, 0xC014BB04, 0xC014BC9C, dma)  # Receive DMA completion wrapper.
    image[start : start + len(code)] = code
    result = (
        image if args.direct_boot is None else replace_direct_kernel(args.direct_boot.read_bytes(), original, image)
    )
    with args.output.open("xb") as output:
        output.write(result)
    print(f"Inserted {len(code)} bytes at {TEXT_PAD:#010x}.")
    print(f"Patched Image SHA-256 {hashlib.sha256(image).hexdigest()}.")


if __name__ == "__main__":
    main()
