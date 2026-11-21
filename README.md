# SRS Revival

Replacement Linux firmware for the **Sony SRS-X7** network module. The X7 plays
local audio through its built-in speakers and keeps Sony's original sound
processing and amplifiers. The local dashboard displays volume and EQ. Two
bootable system slots and an Ethernet rescue system provide recovery if a new
system fails to start.

[Download the firmware source kit](https://github.com/davelindo/srs-revival/releases/tag/x7-source-0.1.0) · [Build instructions](firmware/README.md) · [Owner guide](docs/roadmap.md)

## Playback and installation

The AirPlay 2 receiver accepts Apple Lossless audio over Ethernet and delivers
it to the speaker's digital audio output. Audible AirPlay playback, Wi-Fi
streaming, multiroom playback and Spotify Connect playback are unverified.
Google Cast is unsupported.

The release contains source and an offline packager. A build requires the
original Sony update, the matching Nokia HD-1 kernel and driver files, and
your SSH public key. Installing the resulting image on a factory-stock X7
is unverified. [Required build inputs](firmware/README.md#inputs-for-a-full-build) · [Downloads and licensing](docs/sharing.md)

## Technical reference

[Board and audio path](docs/hardware.md) · [Controller protocol](docs/spi.md) · [Firmware format](docs/update-format.md) · [Emulation](docs/emulation.md) · [Backup and recovery](docs/preservation.md)

[Documentation site](https://davelindo.github.io/srs-revival/) · [Plain-text owner guide](docs/srs-x7.txt) · [Third-party notices](THIRD_PARTY.md) · [Contributing](CONTRIBUTING.md)
