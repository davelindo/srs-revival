# Backup and recovery

## Network NAND formats

The network module has 65,536 NAND pages. Each page has 2,048 data bytes and
64 spare bytes for error correction and other metadata.

| Format | Size | Contents |
| --- | --- | --- |
| Logical NAND | 128 MiB (134,217,728 bytes) | Bootloaders, network firmware, factory settings and per-unit configuration |
| Raw NAND | 132 MiB (138,412,032 bytes) | The same address space with spare bytes for each page |
| Sony BCD | Varies by firmware version | Selected update sections for the stock bootloader |

## Per-unit data

NAND, configuration and RAM can hold Wi-Fi secrets, MAC/serial identifiers,
radio calibration, service tokens and certificates. The Apple authentication
private key is held by the separate authentication component and was not
exported. Device backups and extracted vendor files remain private.

## Separate storage

The Sony controller has separate serial flash. Its downloaded MC update is
not a complete chip dump and may omit boot, reserved or per-unit state.
Bluetooth, touch/NFC and authentication components may contain additional
firmware or protected state. A network NAND backup covers only the network
module.

## Restore format

Raw NAND plus spare requires a programmer or restore tool that handles the
device's geometry, ECC and bad blocks. The BL2 web updater accepts BCD files.
A full-device restore remains untested.

The custom firmware can boot the last confirmed system slot or enter wired
rescue when a new system fails to start. Both recovery paths require intact
bootloaders. Recovery after power loss during a flash write or damage to a
bootloader is unverified.
