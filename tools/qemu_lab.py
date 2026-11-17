"""Load a locally supplied X7 image into paused QEMU for debugger inspection.

This is a CPU/RAM inspection environment, not an SRS-X7 board emulator.
No firmware is distributed, no guest network device exists, and no physical
speaker is contacted. Generated image views live under ignored work/qemu/.
"""

import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tempfile

import bcd
from build_telnet import STOCK_SHA256, PATCHED_SHA256
from download_stock import FILES

MC_COPIES = (
    (0x6000154C, 0x581DB2A4, 0x2414),
    (0x60003960, 0x581DD6B8, 0x14),
    (0x60004000, 0x581DD6CC, 0xF4),
    (0x60008000, 0x581DD7F0, 0x6F7B0),
    (0x60078160, 0x5824CFA4, 0x4AC8),
    (0x601FF400, 0x58251A6C, 0xA14),
)


def image_views(component, blob):
    digest = hashlib.sha256(blob).hexdigest()
    if component == "network":
        if digest not in (STOCK_SHA256, PATCHED_SHA256):
            raise ValueError("requires the pinned stock or verified console-patched X7 BCD")
        app = bcd.sections(blob)[0]["data"]
        bcd.verify_boot(app)
        return "arm926", 0x60008000, [(0x60008000, app[bcd.STAGE2_OFF:])]
    if digest != FILES["Armada_release.bin"]["sha256"]:
        raise ValueError("requires the pinned original X7 Armada update")
    payload = blob[32:]
    ram = bytearray(0x200000)
    for destination, source, size in MC_COPIES:
        offset = source - 0x58100000
        data = payload[offset:offset + size]
        if len(data) != size:
            raise ValueError("controller copy exceeds input")
        ram[destination - 0x60000000:destination - 0x60000000 + size] = data
    return "cortex-a9", 0x58100008, [(0x58100000, payload), (0x60000000, bytes(ram))]


class QMP:
    def __init__(self, process):
        self.process = process
        if "QMP" not in self.read():
            raise ValueError("missing QMP greeting")
        self.call("qmp_capabilities")

    def read(self):
        line = self.process.stdout.readline()
        if not line:
            raise RuntimeError("QEMU exited before QMP response")
        return json.loads(line)

    def call(self, command, **arguments):
        request = {"execute": command}
        if arguments:
            request["arguments"] = arguments
        self.process.stdin.write(json.dumps(request) + "\n")
        self.process.stdin.flush()
        while True:
            response = self.read()
            if "error" in response:
                raise RuntimeError(response["error"])
            if "return" in response:
                return response["return"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("component", choices=("network", "controller"))
    parser.add_argument("image", type=Path)
    parser.add_argument("--serve", action="store_true", help="keep paused QEMU up with a loopback GDB endpoint")
    parser.add_argument("--port", type=int, default=3333)
    args = parser.parse_args()
    if not 1024 <= args.port <= 65535:
        parser.error("port must be between 1024 and 65535")
    executable = shutil.which("qemu-system-arm")
    if executable is None:
        parser.error("qemu-system-arm is not installed")
    blob = args.image.read_bytes()
    cpu, entry, views = image_views(args.component, blob)
    root = Path("work/qemu")
    root.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=root) as temporary:
        temporary = Path(temporary).resolve()
        command = [executable, "-machine", "none", "-cpu", cpu, "-m", "2G",
                   "-accel", "tcg", "-nodefaults", "-no-user-config", "-S",
                   "-display", "none", "-monitor", "none", "-serial", "none",
                   "-nic", "none", "-qmp", "stdio"]
        for index, (address, data) in enumerate(views):
            path = temporary / f"view-{index}.bin"
            path.write_bytes(data)
            command += ["-device", f"loader,file={path},addr={address:#x},force-raw=on"]
        command += ["-device", f"loader,addr={entry:#x},cpu-num=0"]
        if args.serve:
            command += ["-gdb", f"tcp:127.0.0.1:{args.port}"]
        process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                   text=True, encoding="utf-8")
        try:
            qmp = QMP(process)
            state = qmp.call("query-status")
            if state["running"]:
                raise ValueError("inspection guest unexpectedly running")
            for index, (address, data) in enumerate(views):
                path = temporary / f"check-{index}.bin"
                qmp.call("pmemsave", val=address, size=len(data), filename=str(path))
                if path.read_bytes() != data:
                    raise ValueError("QEMU memory differs from the supplied image view")
            print(json.dumps({"component": args.component,
                              "input_sha256": hashlib.sha256(blob).hexdigest(),
                              "cpu": cpu, "entry": hex(entry), "paused": True,
                              "memory_views_verified": [hex(address) for address, _ in views],
                              "gdb": f"127.0.0.1:{args.port}" if args.serve else None,
                              "scope": "CPU/RAM inspection only; no board peripherals or full boot"}, indent=2), flush=True)
            if args.serve:
                try:
                    process.wait()
                except KeyboardInterrupt:
                    pass
        finally:
            if process.poll() is None:
                process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
            process.stdin.close()
            process.stdout.close()


if __name__ == "__main__":
    main()
