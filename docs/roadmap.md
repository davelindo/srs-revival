# Custom firmware on the SRS-X7

Linux replaces the operating system on the X7’s network module. Sony’s Armada
controller still selects the audio source, applies the factory speaker tuning
and drives the amplifiers.

| Feature | SRS-X7 support |
| --- | --- |
| Local playback | Plays through the built-in speakers. |
| AirPlay 2 over Ethernet | Receives buffered Apple Lossless (ALAC) and sends it to the digital audio output. Audible playback from an AirPlay sender is unconfirmed. |
| Sound controls | The local dashboard displays the controller’s volume and EQ. The controller supports five adjustable EQ bands. |
| Standby settings | Linux can read the network standby and automatic standby settings. Overnight standby and wake are unverified. |
| Management | The dashboard uses an owner password. SSH accepts the owner’s Ed25519 key. |
| Recovery | Two bootable system slots support rollback to the confirmed system. Wired rescue is available with intact bootloaders. |

Wi-Fi streaming, AirPlay multiroom synchronization and Spotify Connect
playback are unverified. Google Cast is unsupported.

## Getting the firmware

Version 0.1.0 is a source kit. It builds on Alex Pawlowski’s **zd1** firmware
for the Zeppelin Air and adds the X7 controller protocol, audio routing and
image packaging. Supply your own Sony stock update, matching Nokia kernel
and drivers, and SSH public key. The stock input is SRS-X7 version 2.13.2.09.
A fresh installation on a factory-stock X7 is unverified.

[Download the source kit](https://github.com/davelindo/srs-revival/releases/tag/x7-source-0.1.0) · [Build instructions](https://github.com/davelindo/srs-revival/blob/x7-source-0.1.0/README.md) · [Board and audio path](hardware.md)
