# Local firmware emulation

Firmware images stay on your machine. This repository supplies source tools
and verified loading addresses; it does not host Sony binaries or private NAND.

## What runs where

| Environment | Supported scope |
| --- | --- |
| QEMU image lab in this repository | Load exact X7 network/controller images into a paused CPU/RAM environment, inspect them through GDB, verify memory against the local input |
| Instruction-level checks | Shell-routing, manifest and SPI routines executed with selected OS/peripheral behavior modeled |
| zd1 Renode model | Related DM870 network module and B&W AVR32 board; substantial boot/peripheral support, with documented limitations |
| Complete SRS-X7 emulation | Not established; Sony controller and board integration remain incomplete |

QEMU 11.1.3 has no DM870/SRS-X7 board model. This tool uses the `none` machine
for paused CPU/RAM inspection. NAND, interrupts, timers, GPIO, DMA and network
peripherals are absent.

## Load an image in QEMU

Install `qemu-system-arm` through your operating system's QEMU package. Obtain
the exact stock files with the [download procedure](telnet.md#2-download-and-verify-the-originals).
Then run:

```sh
.venv/bin/python tools/qemu_lab.py network work/stock/Laphroaigv2.09a.bcd
.venv/bin/python tools/qemu_lab.py controller work/stock/Armada_release.bin
```

Each command verifies the input hash, starts paused QEMU, loads the image
views, compares **all loaded bytes** through QEMU's physical-memory interface,
prints the result and exits. Generated views are temporary files under ignored
`work/qemu/`. No physical device or guest network is connected.

To keep an image available to a local GDB-compatible debugger:

```sh
.venv/bin/python tools/qemu_lab.py network \
  work/Laphroaigv2.09a-shell2320.bcd --serve --port 3333
```

Connect from an ARM-capable GDB installation:

```text
target remote 127.0.0.1:3333
x/2wx 0x60266d9c
x/2i 0x60266d9c
```

The patched image should show words `0xe3a05e91 0xe3a04001`. For the original
image they are `0xe1a05002 0xe1a04001`. Stop the host command with Ctrl-C when
finished. The GDB endpoint binds only to loopback. Guest RAM is disposable;
the original input file is never attached as writable storage.

Unmodified startup requires device models and OS-service hooks beyond this
inspection environment.

## Image mappings

| Image | CPU selection | Loaded view |
| --- | --- | --- |
| Network bCoD | `arm926` | Application stage 2, after its `0x2000`-byte preloader, at `0x60008000` |
| Armada | `cortex-a9` | Payload after 32-byte wrapper at `0x58100000`; PC initialized to `0x58100008` |
| Armada initialized RAM | Same controller CPU | Startup-copy views reconstructed in `0x60000000..0x601fffff` |

The generic `none` machine maps its RAM at zero. The inspection lab reserves
a sparse 2 GiB address range so both firmware address views fit. The device
itself has 32 MiB of network-module SDRAM. Controller MMU/cache/TCM state is
outside this inspection model.

The [QEMU generic loader documentation](https://www.qemu.org/docs/master/system/generic-loader.html)
describes loading raw images and setting the initial PC.

## Existing DM870 system model

[zd1's emulator](https://github.com/pawlowskialex/zd1/tree/1cd511c0d37a758d6267675664bf78abd36a8ee6/emu)
uses Renode rather than QEMU. It models the ARM926 network module, NAND/spare
data, interrupt/timer/GPIO/UART blocks, SPI slave, MACB network, DMA and audio
interfaces. Its optional main board is B&W's AVR32, not Sony's controller.

The upstream entry points, from its own checkout, are:

```sh
python3 emu/zemu setup
python3 emu/zemu run stock --keep
```

`setup` downloads zd1's reference inputs, and its build path uses Nix. Its
AVR32 controller model is specific to the B&W board. The SPI test host needs
the X7 CRC extension described in [SPI reading](spi.md).

## Requirements for full X7 hosting

A useful system model needs the network module's boot/memory/peripheral
behavior and a Sony-compatible controller endpoint, connected with the actual
CRC, escaping, notification and handshake behavior. The firmware-derived mode,
clock divider, DMA path and GPIO functions now provide concrete inputs.
The remaining validation includes the P1 clock tree, physical timings,
controller startup/state transitions and audio routing.
