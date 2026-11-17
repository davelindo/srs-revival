# Firmware update format

The update manifest describes separate network-module and main-controller
images.

## Identify the right family

| Device | Category / service | Network image | Combined version |
| --- | --- | --- | --- |
| SRS-X7 | `AU002 / PSK004` | `Laphroaigv2.09a.bcd` | 2.13.2.09 |
| SRS-X9 | `AU002 / PSK003` | `Hibikiv2.10a.bcd` | 2.13.2.10 |

Both families use the filename `Armada_release.bin`, with different contents.
The tools in this repository target the pinned X7 image.

## Vendor acquisition

The X7 requests its metadata from this public Sony endpoint:

```text
http://info.update.sony.net/AU002/PSK004/info/info.xml
```

The validated manifest identifies these direct vendor downloads:

- [Network-module update: Laphroaigv2.09a.bcd](http://info.update.sony.net/AU002/PSK004/contents/0004/Laphroaigv2.09a.bcd)
- [Main-controller update: Armada_release.bin](http://info.update.sony.net/AU002/PSK004/contents/0004/Armada_release.bin)


## ENC0003 and HAS0003

The manifest has a plaintext header, a blank line, and AES ciphertext. Header
identifiers `ENC0003` and `HAS0003` select the observed cipher and digest scheme.
For the tested AU002 manifests, the AES-128-ECB key is:

```text
0123456789abcdeffedcba9876543210
```

The device hashes **all decrypted plaintext bytes**, including trailing
spaces used to pad XML to a multiple of 16 bytes. In pseudocode:

```text
plaintext = AES_128_ECB_decrypt(ciphertext, key)
inner = lowercase_hex(SHA1(plaintext))
digest = lowercase_hex(SHA1(ASCII(inner + "PSK004" + "AU002")))
```

The resulting digest must match the header before parsing or using the XML.
Each distribution's `MAC` field is its file's SHA-1 value in this manifest;
the acquisition tool also checks the recorded SHA-256.

```sh
.venv/bin/python tools/manifest.py work/stock/info.xml
```

Digest validation includes every padding byte. The XML parser accepts trailing
spaces and rejects NUL padding. BL2 upload uses the BCD directly and needs no
modified manifest.

## The bCoD container

The X7 network update is a three-section plaintext bCoD v3 container. Its
header CRC covers the first `0xb0` bytes with its CRC field zeroed. The
section descriptors begin at `0x30`, with `0x20` bytes per descriptor.

| Section | Meaning | Size | NAND destination in update descriptor |
| --- | --- | ---: | --- |
| 0 | Application, including preloader | 6,138,624 bytes | `0x0240000` |
| 1 | Factory configuration attributes (CnE) | 34,124 bytes | Special configuration handling; not a general raw-flash address |
| 2 | Resources | 535,588 bytes | `0x0c40000` |

The application has a `0x2000`-byte preloader before stage 2. Stage 2 runs at
cached address `0x60008000`. Its Fletcher checksum uses little-endian 16-bit
words with two `0xffff` accumulators, folded every 360 words. The outer CRCs
use the standard CRC-32 implemented by `zlib.crc32`.

This description and [the code](https://github.com/davelindo/srs-revival/blob/main/tools/bcd.py)
build on the MIT-licensed [zd1 container tools](https://github.com/pawlowskialex/zd1/tree/1cd511c0d37a758d6267675664bf78abd36a8ee6/tools/zeppelin/fw).

## The console patch

| Runtime address | Original operation | Replacement operation | Little-endian replacement |
| --- | --- | --- | --- |
| `0x60266d9c` | `mov r5,r2` | `mov r5,#2320` | `915ea0e3` |
| `0x60266da0` | `mov r4,r1` | `mov r4,#1` | `0140a0e3` |

The shell-routing function receives a mode and port. Mode 1 is TELNET. These
changes force both arguments inside that function, so later routing calls
cannot return the console to UART. Port 2320 avoids the media server already
using TCP 8000. It also fits a single ARM rotated-immediate instruction, so
the patch requires no trampoline, new code section or changed length.

Only four instruction bytes differ across the two instructions; checksum
repairs bring the total to 20 changed bytes in the 6,709,248-byte image. The
repair order matters:

1. Stage-2 Fletcher value in the application's boot header.
2. Stage-1 CRC, whose covered data includes that header value.
3. Application section CRC in the bCoD header.
4. bCoD header CRC.

The factory configuration and resource sections, layout, product identity,
versions and total length are preserved in the file. The updater's treatment
of persistent user settings is a separate concern. The builder validates all
checksums, limits changed offsets, and checks the complete output hash against
the image that was installed. It refuses to overwrite an existing output.
