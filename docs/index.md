# Sony SRS-X7

Technical documentation and source tools for the SRS-X7 network speaker.
The console procedure targets firmware **2.13.2.09** and enables its existing
KnOS diagnostic shell on **TCP 2320**.

| Documentation | Contents |
| --- | --- |
| [Device architecture](hardware.md) | Processors, board components, control/audio boundaries, memory and storage |
| [SPI reading](spi.md) | Console diagnostics, framing, CRC, register catalogue, driver configuration and handshakes |
| [Enable telnet](telnet.md) | Obtain originals from Sony, build the patch locally, install through BL2 and verify |
| [Firmware format](update-format.md) | Manifest decryption, exact hashes, bCoD sections and checksum repair |
| [Local emulation](emulation.md) | Load images in QEMU for inspection; scope of the related Renode model |
| [Backup coverage](preservation.md) | Network NAND, per-unit data, separate controller and restore limitations |
| [Future firmware](roadmap.md) | Requirements for a Sony-compatible replacement |
| [Sources](sources.md) | Vendor references and open-source tools |

## Supported model

This reference covers the Sony SRS-X7 running stock version **2.13.2.09**.
The controller and the network module have separate firmware and storage.

Firmware images, Sony manuals, extracted resources and private device data
are not hosted here. The tools download originals directly from Sony and
produce local files under ignored directories.

Sony lists the X7 in its [17 November 2026 network-service termination notice](https://www.sony.co.uk/electronics/support/speakers-wireless-speakers/srs-x9/articles/00401117).
This project aims to keep the hardware useful after those services end.
