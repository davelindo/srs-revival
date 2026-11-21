# Inside the SRS-X7

A network audio processor works alongside a separate Sony controller.

## Device overview

The SRS-X7 is a 2014-era portable network speaker with Wi-Fi, Ethernet,
Bluetooth, an analogue input, NFC and an internal rechargeable battery.
Sony's [launch specification](https://www.sony-asia.com/corporate/resources/en_AP/pdf/AP%2807January14%29-UnitingOutstandingDesignandPremiumSoundtheNewWirelessSpeakersandAudioSystemsfromSony.pdf)
describes a 2.1-channel system: two full-range drivers and a subwoofer, with
32 W total rated output. Its network software supports the original
AirPlay/RAOP and DLNA generation and later received Spotify support.

## Functional architecture

The network module requests a playback source over SPI. Stereo audio travels
from its AV2 digital output through input selector IC405 to the SSI3 input on
Sony's Armada controller (IC403). Armada applies the speaker's DSP settings
and feeds the digital power amplifiers. Playback requires Armada to select
the network module as the source and release mute.

```text
Wi-Fi / Ethernet
       |
DM870 network module -- AV2 digital audio --> IC405 --> Armada SSI3
  ARM926 / stock KnOS or custom Linux
  32 MiB SDRAM / 128 MiB NAND
       |
       | SPI: registers, commands, queued notifications
       | network module = slave
       | Sony controller = master
       |
Renesas R7S7200022CFP
  Sony control / T-Kernel
  separate MX25L3235E serial flash
       |
Power, source, mute, controls and battery coordination
```

The controller is the SPI **master**; the network module serves a register
protocol. It can notify the controller, which drains pending messages. Each
CPU has its own address space. The [SPI write-up](spi.md)
explains the five-byte headers, CRCs, escaping and queue semantics.

## Board components

| Component | Part and function |
| --- | --- |
| Network processor | BridgeCo DM870, ARM926 at 240 MHz with Warp/SDRAM at 160 MHz |
| Network memory | 32 MiB SDRAM, 64 KiB on-chip SRAM and 128 MiB NAND |
| IC403, main controller | Renesas R7S7200022CFP, runs Sony’s control software |
| IC401, controller flash | Macronix MX25L3235EM2I-10G, 4 MiB serial flash |
| IC402, Apple authentication | MFI337S3959 authentication component |
| IC406, analogue input | PCM1808PWR analogue-to-digital converter |

The audio routing and component identities come from the Sony SRS-X7 service
manual, main-board diagram on page 29 and parts list on page 67.

## Boot and software responsibilities

The network module's preserved NAND contains an encrypted first-stage
bootloader (BL1), a second-stage loader/updater (BL2), an application,
resources and configuration areas. BL2 can accept the model's bCoD application
update over its web interface. The application preloader occupies `0x2000`
bytes; application stage 2 runs at cached address `0x60008000`.

KnOS/JukeBlox provides the network stacks, media decoder, GoAhead web UI,
attribute namespace, device drivers and diagnostic shell. The firmware's
32 MiB build runs an embedded RTOS.

Sony's controller has its own executable and storage. Static analysis of
`Armada_release.bin` shows a 32-byte update wrapper and a flash execution view
starting at `0x58100000`, with startup copies into controller RAM. The entry
literal targets `0x58100008`. These mappings describe the controller's address
space, independently of the network module's similarly numbered RAM.

The controller coordinates power, charging/battery state, source selection
and host-side control. Static dispatchers process network status, version,
update and AirTunes messages. Monitor initialization reads `TmConsEnable`,
`TmDebugPort` and `TmBaudRate`. Its enabled state and transport are unresolved.

## Network services observed on stock software

Service availability changes with standby, setup mode and runtime state.

| TCP port | Observed role |
| --- | --- |
| 80 | GoAhead setup, device details and update entry; BL2 UI in update mode |
| 1309 | RAOP/AirPlay advertisement |
| 8000 | ContentDirectory/media-server HTTP |
| 8080 | UPnP descriptions and control |
| 10100 | DRC control interface |
| 41824 | UPnP-description HTTP role identified in firmware |
| 41893 | Tandem consumer control; state dependent |
| 50001 | Potential TCP/SPI bridge configuration; not established as an accessible stock register service |
| 60151 | Album-art image-server role identified in firmware |
| 2320 | Diagnostic console after the verified patch |

On stock 2.13.2.09, the local Tandem protocol used by SongPal exposes the
model, firmware version, sound presets, standby settings and battery status.

## Memory and storage map

Byte offsets in the network module's NAND:

| NAND offset | Observed contents |
| --- | --- |
| `0x0000000` | Encrypted BL1; first 128 KiB erase block |
| `0x0020000` | Per-unit identity/calibration and boot-related PersParam records |
| `0x0040000` | Factory CnE |
| `0x0060000`, `0x0080000` | Two user-configuration banks |
| `0x00a0000` | BL2 |
| `0x0240000` | Installed application |
| `0x0c40000` | Resources |
| `0x1300000` | Additional factory-CnE copy |
| `0x2000000` | Network-image staging slot, 10 MiB capacity |
| `0x2a00000` | Main-controller update staging slot, 4 MiB capacity |
| `0x2e00000` | Backup XML slot, 1 MiB capacity |
| `0x2f00000` | Update-status slot, 1 MiB capacity |
| `0x3000000` | CA-bundle slot, 1 MiB capacity |

The NW and MC staging areas hold downloaded update files. The controller's
serial flash is a separate storage device.

## The filesystem-like namespace

The console exposes directories and attributes. The captured namespace has
1,395 effective configuration values; the network image contains 172 web
resources.

```text
network-module view
  web resources        ASP setup/update UI, scripts and assets
  /cfg                 persistent/effective configuration attributes
    CommunicationSettings
    SystemHost
    dev/AvPortFormat/AvPort2
  /static              runtime attributes and hierarchy
  device drivers       network, NAND, SPI, audio and other interfaces
```

## Controls available to the replacement firmware

Armada still owns source selection, master volume, mute, equalization and
power policy. Over the native controller link, Linux can read volume, EQ,
the two standby settings and a coarse battery category. The battery reply
does not report pack voltage, temperature, cycle count or battery health.

The five custom EQ bands are centered at 100 Hz, 330 Hz, 1 kHz, 3.3 kHz and
10 kHz. The controller accepts gains from −6 to +6 dB in 0.6 dB steps.
The dashboard shows the gain requested for each band.
