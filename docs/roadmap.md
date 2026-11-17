# Replacement firmware requirements

## Related platform

[zd1](https://github.com/pawlowskialex/zd1) targets the Bowers & Wilkins
Zeppelin Air on the same broad DM870/KnOS lineage. It provides substantial
boot, networking, codec, audio and receiver code. The reference version is
commit `1cd511c0d37a758d6267675664bf78abd36a8ee6`.

The Sony controller differs from the Zeppelin's AVR32. The current zd1 path
uses its own controller dialect, and the maintainer explains the cost of
supporting stock controllers in [the compatibility discussion](https://github.com/pawlowskialex/zd1/issues/1).
A generic zd1 image or B&W controller patch is not an X7 installer.

## Required work

| Area | Required support |
| --- | --- |
| Main controller | Flash readback, update wrapper and per-unit storage layout |
| Sony SPI protocol | Boot, notification, source, volume, mute and standby state machines |
| Recovery | Restore path for modified boot/application images |
| Network-module boot | Sony controller handshake and power control |
| Audio | PCM output, volume, mute and standby/wake |
| AirPlay 2 | Current sender support, synchronization and control |
| Spotify Connect | Authentication, track-key acquisition and playback |
| Cast | Device authentication and sender compatibility |

Retaining Sony's controller requires implementing its network-side SPI
protocol. The network module also needs a maintained OS and driver base.

## Related devices

The SA-NS400 and Zeppelin are research references. The SRS-X9 shares useful
manifest/platform context but has different firmware. SRS-X77/X88/X99 are
newer, materially different software platforms.

Useful related work includes [Pioneer X-SMC55 research](https://github.com/loop333/pioneer-x-smc55)
and [libretouch's hardware notes](https://libretouch.net/docs/hardware/).
