"""Prepare the SRS-X7 source tree, provision an owner, and assemble an image locally."""

import argparse
import base64
import contextlib
import hashlib
import io
import os
from pathlib import Path
import secrets
import shutil
import struct
import subprocess
import sys
import tarfile
import tempfile


KIT = Path(__file__).resolve().parent
UPSTREAM = "1cd511c0d37a758d6267675664bf78abd36a8ee6"
STOCK_SHA256 = "2f8af043a5058bc368ec99222ca391770ac75aca486719281e71cef03d12e129"
SOURCE_PATHS = (
    ".cargo", "Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "flake.nix", "flake.lock",
    "LICENSE", "NOTICE.md", "protocol", "host", "nix", "module/boot", "module/dm870",
    "module/link", "module/codecs", "module/warp0", "module/os", "tools/zeppelin/lab/ap2send",
)
COMPONENTS = (
    "boot", "busybox", "busybox-rescue", "alsa-lib", "alsa-utils", "wpa_supplicant",
    "apps", "apps-core", "x7-management", "x7-kernel", "llvm",
)


@contextlib.contextmanager
def new_directory(target):
    """Reserve a new private directory exclusively and clean up a failed build."""
    target.mkdir(mode=0o700)
    try:
        yield target
    except BaseException:
        shutil.rmtree(target)
        raise


def register_defaults(stock):
    if hashlib.sha256(stock).hexdigest() != STOCK_SHA256:
        raise ValueError("expected the original SRS-X7 Laphroaigv2.09a.bcd")
    app_offset, = struct.unpack_from("<I", stock, 0x30)
    table = app_offset + 0x2000 + (0x6051C9A4 - 0x60008000)
    records = struct.iter_unpack("<5I", stock[table:table + 128 * 20])
    rows = sorted((register, a & 255, b & 255) for _, register, size, a, b in records if size == 2)
    if len(rows) != 64 or len({register for register, _, _ in rows}) != 64:
        raise ValueError("unexpected SRS-X7 register table")
    return "register,a,b\n" + "".join(f"{r},{a},{b}\n" for r, a, b in rows)


def prepare(args):
    defaults = register_defaults(args.stock.read_bytes())
    # git archive reads the pinned public commit, including when the checkout is dirty.
    source = subprocess.check_output(
        ["git", "-C", args.upstream, "archive", "--format=tar", UPSTREAM, "--", *SOURCE_PATHS]
    )
    with new_directory(args.output) as tree:
        with tarfile.open(fileobj=io.BytesIO(source)) as archive:
            archive.extractall(tree, filter="data")
        # Make patch resolution independent of any surrounding checkout.
        subprocess.run(["git", "init", "--quiet", tree], check=True)
        subprocess.run(["git", "-C", tree, "apply", KIT / "zd1-port.patch"], check=True)
        shutil.rmtree(tree / ".git")
        shutil.copytree(KIT / "port", tree, dirs_exist_ok=True)
        (tree / "x7/management/sony-registers.csv").write_text(defaults, encoding="ascii")
    print(f"Prepared {UPSTREAM} with the X7 port in {args.output}")


def public_key(path):
    fields = path.read_text(encoding="utf-8").strip().splitlines()
    if len(fields) != 1:
        raise ValueError("supply one OpenSSH Ed25519 public key")
    parts = fields[0].split()
    if len(parts) < 2 or parts[0] != "ssh-ed25519":
        raise ValueError("expected ssh-ed25519 public key")
    raw = base64.b64decode(parts[1], validate=True)
    prefix = b"\x00\x00\x00\x0bssh-ed25519\x00\x00\x00\x20"
    if len(raw) != len(prefix) + 32 or not raw.startswith(prefix):
        raise ValueError("malformed Ed25519 public key")
    return "ssh-ed25519 " + base64.b64encode(raw).decode("ascii") + "\n"


def owner(args):
    key = public_key(args.public_key)
    with new_directory(args.output) as directory:
        for name, value in (
            ("authorized_keys", key), ("web-password", secrets.token_urlsafe(32) + "\n"),
            ("CountryCode", args.country + "\n"), ("NetIFMode", args.interface + "\n"),
        ):
            (directory / name).write_text(value, encoding="ascii")
    print(f"Owner files are in {args.output}. The dashboard password is in web-password.")


def parts(args):
    tree = args.tree.resolve()
    with new_directory(args.output) as directory:
        for component in COMPONENTS:
            subprocess.run([
                "nix", "--extra-experimental-features", "nix-command flakes", "build",
                "--no-write-lock-file", "--out-link", directory / component, f"path:{tree}#{component}",
            ], check=True)
    print(f"Production components are in {args.output}")


def pack(args):
    paths = [p.resolve(strict=True) for p in (args.tree, args.parts, args.vendor, args.stock, args.owner)]
    env = os.environ.copy()
    llvm = paths[1] / "llvm/bin"
    if llvm.is_dir():
        env["PATH"] = str(llvm) + os.pathsep + env.get("PATH", "")
    output = args.output.absolute()
    if os.path.lexists(output):
        raise FileExistsError(output)
    # Rename only completed outputs. Failed runs cannot be mistaken for a finished image.
    with tempfile.TemporaryDirectory(dir=output.parent) as tmp:
        bundle = Path(tmp) / "image"
        # The parent is private. Standard Unix modes inside it become the speaker's filesystem.
        subprocess.run([args.image_tool.resolve(strict=True), "assemble", *paths, bundle],
                       env=env, umask=0o022, check=True)
        subprocess.run([sys.executable, "-I", KIT / "verify.py", bundle, paths[-1]], check=True)
        if os.path.lexists(output):
            raise FileExistsError(output)
        bundle.rename(output)
    print(f"Owner image and checksums are in {output}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    prep = sub.add_parser("prepare", help="overlay the port on a pinned local upstream checkout")
    prep.add_argument("--upstream", type=Path, required=True)
    prep.add_argument("--stock", type=Path, required=True)
    owned = sub.add_parser("owner", help="make fresh management credentials and region settings")
    owned.add_argument("--public-key", type=Path, required=True)
    owned.add_argument("--country", required=True, choices=("JP", "US", "EU"))
    owned.add_argument("--interface", default="wired", choices=("wired", "wireless", "autosense"))
    build = sub.add_parser("parts", help="cross-build the production components with Nix on Linux")
    build.add_argument("--tree", type=Path, required=True)
    image = sub.add_parser("pack", help="assemble and verify the owner image offline")
    for name in ("image-tool", "tree", "parts", "vendor", "stock", "owner"):
        image.add_argument("--" + name, type=Path, required=True)
    for cmd in (prep, owned, build, image):
        cmd.add_argument("--output", type=Path, required=True, help="a new directory in an existing parent")
    args = parser.parse_args()
    os.umask(0o077)
    try:
        {"prepare": prepare, "owner": owner, "parts": parts, "pack": pack}[args.command](args)
    except (OSError, ValueError, tarfile.TarError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"kit: {error}\n")


if __name__ == "__main__":
    main()
