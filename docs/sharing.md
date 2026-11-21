# Firmware downloads and licensing

[Download the SRS-X7 source kit](https://github.com/davelindo/srs-revival/releases/tag/x7-source-0.1.0) · [Build instructions](https://github.com/davelindo/srs-revival/blob/x7-source-0.1.0/README.md) · [Plain-text owner guide](srs-x7.txt)

The kit includes the X7 firmware source, the offline packager and the license
notices. You can share the kit or forward the plain-text guide with its notice.

## Files to supply locally

You need the Sony stock update, matching Nokia HD-1 kernel and drivers, and
your SSH public key. The packager embeds that key and a fresh dashboard
password in both the system and rescue images. Keep the built images private.
They carry your access credentials and vendor software licensed separately
from the kit. [Required files and hashes](https://github.com/davelindo/srs-revival/blob/x7-source-0.1.0/README.md#inputs-for-a-full-build)

## Why distribute source?

Service retirement leaves the software licenses in place. Sony's X7 guide
lists terms for individual stock components. For the replacement system,
the concrete gap is the modified Nokia Linux 2.6.24.7 kernel. Its upstream
tree has the binary and lacks the corresponding source. Distributing that
kernel under GPLv2 would require satisfying section 3 for the exact build.
The binary drivers and radio firmware also need redistribution rights.
BusyBox carries GPL obligations, and the statically linked ALSA and receiver
libraries have LGPL notice and relinking requirements.
[Sony SRS-X7 software notices](https://helpguide.sony.net/speaker/srs-x7/v1/en/contents/TP0000389783.html)
· [Upstream component notices](https://github.com/pawlowskialex/zd1/blob/1cd511c0d37a758d6267675664bf78abd36a8ee6/NOTICE.md)
· [GPLv2 §3](https://www.gnu.org/licenses/old-licenses/gpl-2.0.en.html)
· [LGPLv2.1 §6](https://www.gnu.org/licenses/old-licenses/lgpl-2.1.html)
· [LGPLv3 §4](https://www.gnu.org/licenses/lgpl-3.0-standalone.html)

## Legal references

Readers can describe observed behavior and measurements in their own words.
The project's MIT notice accompanies copies of its writing and tools.
[Canadian copyright guide](https://ised-isde.canada.ca/site/canadian-intellectual-property-office/en/guide-copyright)
· [U.S. Copyright Office guidance](https://www.copyright.gov/help/faq/faq-protect.html)

The earlier stock-console material includes a manifest key and a patch. For
those particular tools, the relevant questions are what the mechanism controls
and which interoperability or repair provisions cover supplying the tool.
The laws distinguish working on one's own device from distributing certain
access tools. [Canadian interoperability provision](https://laws-lois.justice.gc.ca/eng/acts/C-42/section-41.12.html)
· [Canadian repair provision](https://laws-lois.justice.gc.ca/eng/acts/C-42/section-41.121.html)
· [U.S. §1201](https://www.copyright.gov/title17/92chap12.html)
· [U.S. repair exemption](https://www.ecfr.gov/current/title-37/chapter-II/subchapter-A/part-201/section-201.40#p-201.40(b)(15))
· [EU Software Directive, Articles 5–6](https://eur-lex.europa.eu/eli/dir/2009/24/oj/eng)
