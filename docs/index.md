# Sony SRS-X7

The custom firmware runs Linux on the X7’s network module and plays local audio
through the built-in speakers with Sony’s original tuning. The public release
contains the source and offline packaging tools. Installation on a factory-stock
speaker is unverified.

[Download the source kit](https://github.com/davelindo/srs-revival/releases/tag/x7-source-0.1.0) · [Playback, controls and recovery](roadmap.md) · [Build instructions](https://github.com/davelindo/srs-revival/blob/x7-source-0.1.0/README.md)

| Reference | What you’ll find |
| --- | --- |
| [Firmware build guide](https://github.com/davelindo/srs-revival/blob/x7-source-0.1.0/README.md) | Required files and hashes, owner credentials and offline packaging |
| [Hardware](hardware.md) | Board components, the audio signal path and memory layout |
| [Controller protocol](spi.md) | SPI frames, checksums, registers and handshakes |
| [Firmware format](update-format.md) | Update containers and their sections |
| [Emulation](emulation.md) | QEMU image inspection and the Renode device model |
| [Backup and recovery](preservation.md) | NAND formats, per-unit data and recovery limits |
| [Downloads and licensing](sharing.md) | Source, third-party inputs and what owners can share |
| [Sources](sources.md) | Sony documentation and upstream projects |

Sony lists the SRS-X7 among the models affected by its 17 November 2026
network-service retirement. Local playback and non-network functions are
covered separately in its notice. [Sony service notice](https://www.sony.co.uk/electronics/support/speakers-wireless-speakers/srs-x9/articles/00401117)
