# Sources, credits and licensing

SRS Revival is an independent project. Sony manufactures the speaker.

The repository's MIT license covers its original writing and tooling. It does
not license Sony firmware, manuals, apps, service responses, trademarks, or
material a reader downloads separately. Downloaded and patched firmware remain
outside the published repository and website. No Sony binaries, extracted
filesystems, decompiled source, manual scans, or device backups are distributed.

## zd1

[Alex Pawlowski's zd1](https://github.com/pawlowskialex/zd1) is the essential
BridgeCo/KnOS reference and inspiration for the longer-term revival work.
`tools/bcd.py` adapts the container and boot-checksum routines from
`tools/zeppelin/fw/bcd.py` at commit
`1cd511c0d37a758d6267675664bf78abd36a8ee6`. It retains the upstream algorithm
and adds input bounds checks; unrelated partition installation/extraction
functions are omitted. The upstream MIT notice is retained in
[LICENSES/zd1.txt](LICENSES/zd1.txt) and the source header.

The telnet patch follows the successful SRS-X7 research implementation. The
manifest and patch explanations are independently written descriptions of the
observed formats and procedure. Small instruction signatures identify the
specific modification; this repository does not contain a firmware payload.

PyCryptodome and Python-Markdown are installed dependencies, not vendored here;
their respective licenses apply to those packages.
