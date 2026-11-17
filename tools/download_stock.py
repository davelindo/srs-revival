"""Download stock X7 update inputs directly from Sony, then check pinned hashes.

Creates private local files only. Never contacts a speaker or installs anything.
"""

import argparse
import hashlib
import http.client
import json
from pathlib import Path

from manifest import decode

HOST = "info.update.sony.net"
PREFIX = f"http://{HOST}/AU002/PSK004/contents/0004/"
FILES = {
    "Laphroaigv2.09a.bcd": {
        "bytes": 6709248,
        "sha1": "c1fcc8b2a3aaff12b13c44d9b072c856e1eff15f",
        "sha256": "2f8af043a5058bc368ec99222ca391770ac75aca486719281e71cef03d12e129",
    },
    "Armada_release.bin": {
        "bytes": 3129408,
        "sha1": "a5a3be71ee193bb828813da49e98d750f790deda",
        "sha256": "7a506cb1794649b56e453c98f3c47f4a328c6c13b6f51d3abea3b9be248e07cc",
    },
}


def fetch(path, limit):
    connection = http.client.HTTPConnection(HOST, timeout=30)
    try:
        connection.request("GET", path, headers={"User-Agent": "srs-revival-research/1"})
        response = connection.getresponse()
        if response.status != 200:
            raise ValueError(f"Sony returned HTTP {response.status} for {path}")
        data = response.read(limit + 1)
        if len(data) > limit:
            raise ValueError("download exceeds expected size")
        return data
    finally:
        connection.close()


def verify_file(name, data):
    record = FILES[name]
    if len(data) != record["bytes"]:
        raise ValueError(f"{name}: size mismatch")
    for algorithm in ("sha1", "sha256"):
        if hashlib.new(algorithm, data).hexdigest() != record[algorithm]:
            raise ValueError(f"{name}: {algorithm} mismatch")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=Path("work/stock"))
    args = parser.parse_args()
    manifest = fetch("/AU002/PSK004/info/info.xml", 65536)
    _, root = decode(manifest)
    distributions = list(root.iter("Distribution"))
    pending = {"info.xml": manifest}
    for name, record in FILES.items():
        uri = PREFIX + name
        matches = [node for node in distributions if node.get("URI") == uri]
        if not matches or any(int(node.get("Size", "0")) != record["bytes"] or
                              node.get("MAC") != record["sha1"] for node in matches):
            raise ValueError(f"manifest no longer describes the pinned {name}")
        data = fetch(uri.removeprefix(f"http://{HOST}"), record["bytes"])
        verify_file(name, data)
        pending[name] = data
    # Validate every download before writing any file; refuse to replace baselines.
    args.output.mkdir(parents=True, exist_ok=True)
    if any((args.output / name).exists() for name in pending):
        raise FileExistsError("use a new output directory; an input already exists")
    for name, data in pending.items():
        with (args.output / name).open("xb") as target:
            target.write(data)
    print(json.dumps({"output": str(args.output), "verified": FILES}, indent=2))


if __name__ == "__main__":
    main()
