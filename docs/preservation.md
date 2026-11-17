# Backup coverage

## Network NAND formats

The network module has 65,536 pages of 2,048 data bytes and 64 spare bytes
each. A logical image is 128 MiB. A raw image with spare bytes is 132 MiB.

## Per-unit data

NAND, configuration and RAM can hold Wi-Fi secrets, MAC/serial identifiers,
radio calibration, service tokens and certificates. The Apple authentication
private key is held by the separate authentication component and was not
exported. Device backups and extracted vendor files remain private.

## Separate storage

The Sony controller has separate serial flash. Its downloaded MC update is
not a complete chip dump and may omit boot, reserved or per-unit state.
Bluetooth, touch/NFC and authentication parts may have further firmware or
protected state. Boot ROM, pre-application decrypted BL1 state and internal
DSP memories were not completely captured.

The network flash driver's second address range maps RAM through a memcpy
driver. Cached and uncached RAM addresses are aliases of the same memory.

## Restore format

Raw NAND plus spare requires a programmer or restore tool that handles the
device's geometry, ECC and bad blocks. The BL2 web updater accepts BCD files.
A full-device restore remains untested.

The [architecture](hardware.md) and [SPI investigation](spi.md) explain why
the controller remains a separate preservation target.
