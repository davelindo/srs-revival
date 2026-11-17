# Sources and provenance

## Vendor references

- [Sony UK: network-service termination, 17 November 2026](https://www.sony.co.uk/electronics/support/speakers-wireless-speakers/srs-x9/articles/00401117).
  Includes SRS-X7; distinguishes affected online features from local playback.
- [Sony Canada: network-service termination notice](https://www.sony.ca/en/electronics/support/speakers-wireless-speakers/srs-x7/articles/00400542).
  Regional notice with the same effective date and an explicit model list.
- [Sony: firmware update for SRS-X9/SRS-X7](https://www.sony.co.uk/electronics/support/downloads/00014810).
  Establishes the 2017 combined version identifiers and network-update context.
- [Sony Help Guide: updating the software](https://helpguide.sony.net/speaker/srs-x7/v1/en/contents/TP0000302405.html).
  AC-power and uninterrupted-update requirements.
- [Sony 2014 launch release and specifications](https://www.sony-asia.com/corporate/resources/en_AP/pdf/AP%2807January14%29-UnitingOutstandingDesignandPremiumSoundtheNewWirelessSpeakersandAudioSystemsfromSony.pdf).
  Product generation, interfaces and 2.1-channel speaker system.
- [Sony service-manual parts list, hosted by ManualsLib, page 67](https://www.manualslib.com/manual/3362414/Sony-Srs-X7.html?page=67).
  Factual identities for IC401/402/403/406. No manual or schematic is hosted here.

Vendor download paths and file hashes are listed in [the update format](update-format.md).

## Open-source foundations

- [QEMU generic loader](https://www.qemu.org/docs/master/system/generic-loader.html)
  and [empty-machine implementation](https://github.com/qemu/qemu/blob/master/hw/core/null-machine.c).
  Used for the paused local image inspection environment.

- [zd1 by Alex Pawlowski](https://github.com/pawlowskialex/zd1), particularly its
  [pinned container tools](https://github.com/pawlowskialex/zd1/tree/1cd511c0d37a758d6267675664bf78abd36a8ee6/tools/zeppelin/fw)
  and [SPI research](https://github.com/pawlowskialex/zd1/blob/1cd511c0d37a758d6267675664bf78abd36a8ee6/docs/protocol/spi-registers.md).
  The X7 adds a CRC layer, so its transport is not identical.
- [zd1 compatibility/maintenance discussion](https://github.com/pawlowskialex/zd1/issues/1)
  and [hardware/controller discussion](https://github.com/pawlowskialex/zd1/issues/2).
  Controller and board compatibility details.
- [Pioneer X-SMC55 research](https://github.com/loop333/pioneer-x-smc55) and
  [libretouch hardware research](https://libretouch.net/docs/hardware/).
  Related platforms, with different boards and scope.

Adapted code and its retained MIT notice are identified in the repository's
[third-party notices](https://github.com/davelindo/srs-revival/blob/main/THIRD_PARTY.md).

For project questions and corrections, use the public issue tracker.
[SRS Revival issues](https://github.com/davelindo/srs-revival/issues)
