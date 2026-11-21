# Sources, credits and licensing

SRS Revival is an independent project. Sony is the manufacturer of
the speaker.

The repository's MIT license covers its original writing and tooling. It does
not license Sony firmware, manuals, apps, service responses, trademarks, or
material a reader downloads separately. Downloaded and patched firmware remain
outside the published repository and website. No Sony binaries, extracted
filesystems, decompiled source, manual scans, or device backups are distributed.

## zd1

[Alex Pawlowski's zd1](https://github.com/pawlowskialex/zd1) is the
BridgeCo/KnOS reference and the upstream project for the SRS-X7 Linux port.
`tools/bcd.py` adapts the container and boot-checksum routines from
`tools/zeppelin/fw/bcd.py` at commit
`1cd511c0d37a758d6267675664bf78abd36a8ee6`. It retains the upstream algorithm
and adds input bounds checks; unrelated partition installation/extraction
functions are omitted. The upstream MIT notice is retained in
[LICENSES/zd1.txt](LICENSES/zd1.txt) and the source header.

The private X7 image additionally uses a patched Nokia Linux kernel,
binary drivers and radio firmware, and GPL and LGPL userspace components.
The public source kit carries the project's MIT notice, the upstream MIT notice
for the zd1 adaptations and GPLv2 for the Rust SPI kernel module. Its packager
takes the vendor files from the owner. Separate terms govern the components
in a finished image. [Firmware source and notices](firmware/README.md)
[Pinned upstream component notices](https://github.com/pawlowskialex/zd1/blob/1cd511c0d37a758d6267675664bf78abd36a8ee6/NOTICE.md)
· [Firmware availability and sharing](docs/sharing.md)

PyCryptodome and Python-Markdown are installed dependencies, not vendored here;
their respective licenses apply to those packages.
