# Reading the Sony SPI interface

The Sony controller is the host. The network module serves registers, replies
and queued notifications. The X7 adds CRC bytes to the related BridgeCo framing.

## What we can read now

The patched KnOS console exposes the **network side** of the SPI connection.
Two existing diagnostic commands are useful:

```text
reg print
spilog
```

Run them through your single console session after following the
[telnet guide](telnet.md). Save responses under an ignored private directory,
such as `private/spi/`, using your terminal client's capture facility.
`reg print` can include network identifiers and credentials: never paste the
complete output into a public issue. In particular, decimal registers
95–102 (`0x5f..0x66`) concern wireless credentials/configuration.

`reg print` reports module-side register names, capacities, current lengths
and values. `spilog` contains decoded headers and 16-bit timestamps. Payload,
CRC and escape bytes are absent from this log.

## Reproduce the register catalogue locally

Once you have downloaded the pinned original X7 BCD:

```sh
.venv/bin/python tools/catalog_registers.py \
  work/stock/Laphroaigv2.09a.bcd \
  --output work/registers.json
```

The stock network application contains 128 descriptors at runtime addresses
`0x6051c9a4..0x6051d3a4`, each 20 bytes. Each descriptor includes a name
pointer, register ID, capacity and two additional words. The last two words
are preserved without assigning speculative meaning. The tool verifies the
stock hash, section CRCs and string bounds, then writes the catalogue locally;
it does not connect to the speaker.

The original live dump added `0x40`, `0xbe` and `0xdc`, producing a combined
catalogue of 131 registers, with 112 present in the captured runtime state.

## Transport framing

X7 headers contain **five decoded bytes**:

```text
type | register | byte2 | byte3 | header CRC-8
```

The non-reflected CRC-8 uses polynomial `0x85`, initial value zero and no final
XOR. Header CRC covers the first four bytes. Data bodies have a separate CRC
over body bytes. Variable lengths are little-endian. A fixed register's two
data bytes must retain their wire order; interpreting them as an integer
requires a register-specific definition.

| Operation | Host request before escaping | Module reply before escaping |
| --- | --- | --- |
| Fixed write | `00 reg 00 00 HCRC data0 data1 DCRC` | `00 reg 00 00 HCRC` |
| Fixed read | `01 reg 00 00 HCRC` | `01 reg 00 00 HCRC data0 data1 DCRC` |
| Variable write | `02 reg lenLo lenHi HCRC data… DCRC` | `02 reg lenLo lenHi HCRC` |
| Variable read | `03 reg maxLo maxHi HCRC` | `03 reg lenLo lenHi HCRC data… DCRC` |

Both directions escape the full frame, including CRC bytes:

| Decoded byte | On the wire |
| --- | --- |
| `0x27` | `27 27` |
| `0xff` | `27 01` |
| Idle | Raw `ff`; not protocol data |

The table covers ordinary operations. Error/retry traffic is additional: the
network parser reports type `0x0d` with code `0xe0` for header failure and
`0xe1` for body failure. The offline decoder supports ordinary read/write
types `0x00..0x03`.

## Decode captured wire frames offline

The source tool is a frame decoder and log summarizer; **it has no network or
hardware transport**. Supply one complete wire frame, not a `spilog` row:

```sh
.venv/bin/python tools/spi.py decode --direction request --hex '01 11 00 00 7f'
.venv/bin/python tools/spi.py summarize private/spi/spilog.txt
```

The first command is a fixed-read request for register `0x11`, including its
header CRC. For replies use `--direction reply`. The decoder unescapes bytes,
validates the header and any data CRC, checks exact frame length, and emits
the register, type, length and body. Keep decoded bodies private too.

The summarizer accepts six decimal columns: timestamp, marker, operation,
register, header byte 2 and header byte 3. It prints counts by type/register.

## Notification queue

- A read of `0x00` dequeues the oldest pending message when one exists.
- A read of `0x0c` reports the register ID at the head of the pending queue.
- A read of a specific register can return and remove its queued message.
- The request GPIO is deasserted when the queue becomes empty.

## Register groups and Sony-side evidence

| Registers | Established role or useful boundary |
| --- | --- |
| `0x05..0x08` | Stream/audio format, channel and frequency attributes |
| `0x11` | Module status; controller saves first byte and signals an event |
| `0x25` | Module version; Sony copies up to 20 bytes and signals version event |
| `0x26..0x27`, `0x90` | Additional boot/version/configuration identities |
| `0x32..0x3f`, `0x5a..0x66` | Network state and wireless configuration; private values |
| `0x40`, `0x44..0x4a` | Host commands, notifications, configuration and DRC |
| `0x75..0x79`, `0x7c` | Shell routing and communication/bridge settings |
| `0x7e` | Update-control data; handled by a distinct Sony parser |
| `0xe1..0xe6` | Power-management and Apple/AirPlay-related state |
| `0xf7` | Battery-status request name; stored `00 00` is not a measured charge |
| `0xf9..0xfa` | Spotify command/status interfaces |

During the controller's initial pending handshake, status `0x21` passes 0 to
the completion callback; `0x20` and `0x23` pass 1. Separate battery handlers map
capacity enums to 100/80/60/40/20/10/0 percent labels. The documented CLI queries
have not returned a live battery reading.

## Static reconstruction anchors

These addresses refer only to the exact pinned X7 firmware pair:

| Processor | Address | Role |
| --- | --- | --- |
| Network module | `0x6026f814` | Receive/parser, CRC checking and notification dequeue |
| Network module | `0x6026f5bc` | Reply construction |
| Network module | `0x602a1914` | CRC-8 routine |
| Network module | `0x601e65fc` | SPI transport and GPIO interface initialization |
| Network module | `0x6029a8f8` | Register lookup, length check and callback/notification |
| Sony controller | `0x58154d60` | CRC-8 routine |
| Sony controller | `0x58154dd0` | Request builder |
| Sony controller | `0x58154eee`, `0x58154fa2` | Header/reply and body validation |
| Sony controller | `0x5815740e` | Transaction state dispatcher |
| Sony controller | `0x58157632` | Stuffing and RSPI submission |
| Sony controller | `0x58156644` | Register-reply dispatcher |

Both CRC routines, byte stuffing and four request-builder operations were
checked under Unicorn with modeled memory.

## RSPI configuration and handshakes

Configuration from the controller wrapper and RSPI setup routines, executed
against modeled MMIO with kernel utilities stubbed:

| Property | Firmware-derived value |
| --- | --- |
| RSPI instance | Channel 0, register base `0xe800c800` |
| Role | Master |
| Mode | 3: CPOL=1, CPHA=1 |
| Transfer unit | 8 bits, MSB first |
| Clock | `P1_clock_hz / 66`; absolute P1 frequency not yet established |
| `SPCMD0` | `0xe403` |
| `SPBR` | `0x20` (32) |
| `SPDCR` | `0x20` |
| `SPCR` after setup | `0x48` |
| TX DMA | Channel 6, from source RAM to RSPI data register `0xe800c804` |
| RX DMA | Channel 5, from `0xe800c804` to destination RAM |
| Transaction select | Controller `P6_1`, via `PSR6` at `0xfcfe3118` |
| Pending request input | `P5_5`, bit 5 of `PPR5` at `0xfcfe3214` |
| Response handshake input | `P5_6`, bit 6 of `PPR5` at `0xfcfe3214` |

The actual wrapper at `0x58170578` supplies configuration words
`[8, 0, 0, 1, 66, 2, 6, 5]`. Driver installation selects callbacks
`0x6005d0d3`, `0x6005d05b`, `0x6005d12b`, `0x6005d11d`.

### Completion and cancellation

| Path | Established behavior |
| --- | --- |
| TX DMA request | CHCFG `0x00200268`, peripheral request `0x121` |
| RX DMA request | CHCFG `0x00100260`, peripheral request `0x122` |
| FIFO reset | `SPBFCR=0xc0`, then `0x30` |
| TX completion at `0x6005c7c2` | Stops TX DMA and wakes the worker; the worker also waits for `SPSR.TEND` before reporting completion |
| RX completion at `0x6005c82e` | Stops RX DMA and reports completion |
| Application callback at `0x581705b4` | Counts two completions and signals its event after both |
| Cancel at `0x6005d11c` | Stops both DMA sides and disables requests |

The driver's OS interrupt numbers 302/303/304 map to GIC IDs 270/271/272 through
the `0x600156ca` translation. Its selected DMA backend differs from the generic
FIFO/interrupt routines also present in the image.

### GPIO levels

Transaction selection writes `0x00020000` for low and `0x00020002` for high
to `PSR6`. P5_5 high selects pending-notification handling. P5_6 is awaited low
before request transmission and high before response reception.

These names are SoC port identifiers. Board connector pins and the absolute
SPI clock frequency remain unresolved.

Register definitions and the clock relationship are documented in the
[Renesas RZ/A1H/A1M hardware manual, Rev. 7.00](https://www.renesas.com/en/document/mah/rza1h-group-rza1m-group-users-manual-hardware),
sections 6, 9, 16 and 54. The values above come from the pinned Sony image.

## Transaction state machine

The controller transaction dispatcher has 15 states covering queue selection,
request handshake, transfer completion, delay, response handshake, header and
body reception, validation, retry, result dispatch and cleanup:

| State | Handler | Role |
| --- | --- | --- |
| 0 | `0x581573b2` | Choose notification/queued command, construct request |
| 1 | `0x58157386` | Wait for input handshake, select transaction |
| 2 | `0x58157350` | Submit request |
| 3 | `0x58157324` | Wait for completion/timeout |
| 4 | `0x58157304` | Release selection, choose delay |
| 5 | `0x581572ee` | Inter-transaction delay |
| 6 | `0x581572a8` | Wait for response handshake, retry on timeout |
| 7 | `0x58157292` | Prepare header reception |
| 8 | `0x5815725c` | Wait for header |
| 9 | `0x5815714c` | Validate header/error/ack, choose body handling |
| 10 | `0x58157110` | Receive body and CRC |
| 11 | `0x58157060` | Validate body CRC, dispatch or retry |
| 12 | `0x58157036` | Prepare retry |
| 13 | `0x58157020` | Dispatch result |
| 14 | `0x58156f9e` | Finish/error cleanup |

Static timer constants include 3000, 1000, 100 and 5 ticks. Their tick rate and
physical deadlines still require validation.

## Remaining measurements

The P1 clock source/frequency, countdown timer tick and higher-level
boot/source/volume/power states remain partly unresolved. No controller-flash
read command has been identified on this SPI interface.
