# Enable the KnOS console

Build a console-enabled image from Sony's stock firmware, then install it
through the BL2 web updater. This procedure targets SRS-X7 firmware 2.13.2.09.

## Read before installing

**Installing this image writes the network module's application flash.** A
power interruption, unsupported revision or image that cannot boot can leave
a unit unusable. Full-device restore remains untested.

This was observed on **one SRS-X7**, combined version **2.13.2.09**, with
network version 2.09 and main-controller version 2.13. The enclosure stayed
closed. The patcher requires an exact stock image hash; it refuses other files.
The SRS-X9's **Hibiki** image and newer SRS-X77/X88/X99 images are different
products. Do not install them on an X7.

The resulting console has no authentication and no encryption. Use an isolated
or otherwise trusted LAN, keep TCP 2320 unreachable from the Internet, and
allow only the maintenance client to reach it. It is a KnOS diagnostic
console, with commands capable of writing memory and flash, not a Linux shell.
Keep one console or transfer session open at a time.

## 1. Identify the unit

Find the speaker's address using your router's lease list or its existing
setup page. Set the example to **your** unit's address:

```sh
SPEAKER_IP=192.0.2.7
curl --fail --max-time 10 "http://$SPEAKER_IP/1000/device_details.asp"
```

Replace `192.0.2.7` with the speaker's actual address. Confirm the page
identifies **SRS-X7** and **2.13.2.09**. Save the unit's configuration
and any existing backups privately. If it is on a different release or the
identity is unclear, stop before installing; this guide supplies no tested
upgrade/downgrade path from another baseline.

Sony's [original update instructions](https://www.sony.co.uk/electronics/support/downloads/00014810)
identify the same combined X7 version. Use the AC adaptor and a stable network
throughout the update. [Sony's help guide](https://helpguide.sony.net/speaker/srs-x7/v1/en/contents/TP0000302405.html)
also requires AC power and uninterrupted connectivity. Be prepared to restore
network settings if the updater resets user configuration.

## 2. Download and verify the originals

From the repository root, with Python 3.11 or newer:

```sh
python3 -m venv .venv
.venv/bin/python -m pip install -r requirements.txt
.venv/bin/python tools/download_stock.py --output work/stock
```

The tool retrieves Sony's encrypted manifest, checks its digest, confirms the
expected distribution entries, and downloads both original components from
`info.update.sony.net`. It checks size, manifest SHA-1 and an independently
recorded SHA-256 before saving anything. It refuses to overwrite files.
`Armada_release.bin` is retained as an original research input; **this
procedure does not install or modify the main-controller update**.

| Input | Bytes | SHA-256 |
| --- | ---: | --- |
| `Laphroaigv2.09a.bcd` | 6,709,248 | `2f8af043a5058bc368ec99222ca391770ac75aca486719281e71cef03d12e129` |
| `Armada_release.bin` | 3,129,408 | `7a506cb1794649b56e453c98f3c47f4a328c6c13b6f51d3abea3b9be248e07cc` |

The [update-format page](update-format.md) gives the vendor URLs and digest
formula. Downloads use Sony's legacy HTTP service and are checked against
the pinned SHA-256 values above.

## 3. Build the patched image locally

```sh
.venv/bin/python tools/build_telnet.py \
  work/stock/Laphroaigv2.09a.bcd \
  work/Laphroaigv2.09a-shell2320.bcd
```

Expected output:

```text
bytes: 6709248
console_port: 2320
changed_bytes_including_checksums: 20
sha256: 9afe62ce41d6d2b5d1834d5adcffd46552a6841f25165e3f04d532de751804d2
```

Two instruction substitutions force the routing function to select TELNET and
port 2320 whenever called, including calls from the main controller. The
builder repairs four checksum fields and verifies the final file against the
exact image previously installed. Configuration and resources remain identical
in the container. See [patch details](update-format.md#the-console-patch).

## 4. Enter the BL2 web updater

Open `http://YOUR-SPEAKER-IP/1000/software_update.asp` in your own browser and
use its update control to enter update mode. On the tested unit, the page
submitted a GET to `/goform/formPrepareFirmwareUpdateHandler`, with the
`button` value containing two spaces, `Update`, then two spaces.

Command-line request:

```sh
curl --get --max-time 60 \
  --referer "http://$SPEAKER_IP/1000/software_update.asp" \
  --data-urlencode 'button=  Update  ' \
  "http://$SPEAKER_IP/goform/formPrepareFirmwareUpdateHandler"
```

Allow about 40 seconds for the reboot, then open `http://YOUR-SPEAKER-IP/`.
The BL2 page has a firmware file picker. If it is unavailable, check power
and the router's address lease before proceeding.

## 5. Upload, inspect, then confirm

Use the BL2 page's file picker to select your locally built
`Laphroaigv2.09a-shell2320.bcd`. The observed page posted multipart data to
`/goform/formPostHandler`: file field `appFirmware`, plus submit field
`appFirmware=Update`. A command-line equivalent for **the upload step** is:

```sh
curl --max-time 120 \
  --form 'appFirmware=@work/Laphroaigv2.09a-shell2320.bcd' \
  --form-string 'appFirmware=Update' \
  "http://$SPEAKER_IP/goform/formPostHandler"
```

The confirmation page should display current version 2.09 and new version
2.09. BL2 accepts this same-version update.

Use the **actual confirmation form** to proceed. Its observed target was
`/goform/formDownloadConfirmationHandler`, with GET parameter `button` equal
to six spaces, `OK`, then six spaces. Selecting this button starts flashing.

Wait for progress to reach 100% and the unit to reboot. Keep power connected
throughout. Recheck the router's lease if the address changes.

## 6. Verify the console and instructions

With a telnet client installed, connect to your unit on the explicit port:

```sh
telnet "$SPEAKER_IP" 2320
```

Submit commands with **CRLF**. Use your client's CRLF mode if needed; LF alone
does not reliably submit a KnOS command. Run these two read-only checks:

```text
sys ver
rd 0x60266d9c 2
```

The version response should identify the KnOS/JukeBlox/DM870 platform. The
second command must return the two 32-bit words:

```text
0x60266d9c: 0xe3a05e91 0xe3a04001
```

## After access

Read [preservation](preservation.md) before exploring flash commands. A NAND
backup may contain Wi-Fi credentials, calibration, identity and certificates.
Keep it private. A backup taken after installation includes the patched
application.

Stock rollback through BL2 remains untested. Restoring raw NAND requires
separate tooling for spare data, ECC and bad blocks.
