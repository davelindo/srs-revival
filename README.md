# SRS Revival

Technical documentation and source tools for the Sony SRS-X7: architecture,
SPI, firmware acquisition and access to its existing KnOS diagnostic console.
The aim is to keep the hardware useful after its network services end and
provide a foundation for maintainable replacement firmware.

**[Documentation](https://davelindo.github.io/srs-revival/)** ·
**[Enable the console](docs/telnet.md)** · **[Hardware](docs/hardware.md)**

Sony lists the SRS-X7 in its [17 November 2026 network-service termination
notice](https://www.sony.co.uk/electronics/support/speakers-wireless-speakers/srs-x9/articles/00401117).
Local playback and non-network functions are separate from the retired services.

## What works

The network module on stock firmware **2.13.2.09** uses KnOS. Its 128 MiB
NAND stores the bootloaders, application and per-unit configuration.

This is a diagnostic RTOS console. AirPlay 2, updated Spotify Connect and a
modern Cast receiver have **not** been demonstrated on this X7. A complete
device restore and the separate main-controller backup remain unverified.

## Reproduce the local build

Requires Python 3.11 or newer. These commands download files from Sony and
create a patched file on your computer; they do not contact a speaker.

```sh
python3 -m venv .venv
.venv/bin/python -m pip install -r requirements.txt
.venv/bin/python tools/download_stock.py --output work/stock
.venv/bin/python tools/build_telnet.py \
  work/stock/Laphroaigv2.09a.bcd \
  work/Laphroaigv2.09a-shell2320.bcd
```

Read the [full installation guide](docs/telnet.md) before proceeding. Installing
changes persistent flash and can disable an unrecoverable unit. The patcher
accepts one exact stock SHA-256 and reproduces the image already tested; it
does not adapt a patch to another model or release. Downloads depend on Sony
continuing to serve the originals. There is no bundled or mirrored fallback.

## Documentation

| Read | Contents |
| --- | --- |
| [Overview](docs/index.md) | Device documentation and tools |
| [Telnet procedure](docs/telnet.md) | Download, build, install and verify the console |
| [Update format](docs/update-format.md) | Manifest validation, bCoD structure and checksum chain |
| [Hardware](docs/hardware.md) | Processors, board functions, memory and separate storage domains |
| [SPI reading](docs/spi.md) | Console diagnostics, framing/CRC, offline decoding and register catalogue |
| [Local emulation](docs/emulation.md) | QEMU image inspection and the related Renode system model |
| [Preservation](docs/preservation.md) | Backup coverage, private data and recovery limits |
| [Future firmware](docs/roadmap.md) | Sony SPI, boot, audio and receiver requirements |
| [Sources](docs/sources.md) | External references, provenance and attribution |

## Publication boundary

The repository and website contain original explanations, source tools,
hashes, and links. They contain **no firmware binaries, patched images,
Sony manuals, APKs, decompiled Sony code, extracted resources, or private
device dumps**. Build your own image from files downloaded directly from Sony.
The `work/` and `private/` directories are ignored. CI checks the files tracked
by Git and builds the website without downloading firmware.

SRS Revival is an independent project. [Third-party notices](THIRD_PARTY.md)
cover the zd1 code used for container validation.

## Develop the site

```sh
.venv/bin/python -m unittest discover -s tests -v
.venv/bin/python tools/check_publication.py
.venv/bin/python tools/build_site.py
.venv/bin/python -m http.server 8765 --bind 127.0.0.1 --directory _site
```

The Markdown in `docs/` is the website source. The small builder uses
Python-Markdown and the template/styles in `site/`; `_site/` is generated and
ignored. GitHub Actions runs the checks and deploys that directory to Pages
on pushes to `main`. Pull requests build and validate without deploying.
