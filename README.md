# SRS-X7 firmware source kit 0.1.0

This kit contains the Sony SRS-X7 port of the **zd1** Linux firmware and the
tools to assemble a system, a wired rescue image and an update container on
your own computer. Both images use credentials you supply or generate locally.
A fresh installation on a factory-stock speaker is unverified.
[Playback, controls and recovery](https://davelindo.github.io/srs-revival/roadmap.html)

| Source | What it does |
| --- | --- |
| `port/x7/protocol`, `transport`, `registers`, `control` | Frame, check and exchange messages with Sony’s original main controller over SPI. |
| `port/x7/kernel` | Implement the SPI kernel module and repair the retained kernel’s Ethernet receive ring. |
| `port/x7/management` | Handle controller startup, volume, EQ, power, the local dashboard and SSH public-key authentication. |
| `zd1-port.patch` and `port/module` | Adapt the pinned **zd1** boot stub, audio output, services and A/B slot selection to the X7. |
| `port/x7/image`, `kit.py`, `verify.py` | Prepare sources, provision an owner, assemble the three images and check their contents. |

## Read or test the controller protocol

Run these commands from the extracted kit. The framing tests use Rust 1.95
or newer and can run offline with no speaker, firmware or external crates.

```sh
rustc --edition=2024 --test port/x7/protocol/src/lib.rs -o /tmp/x7-protocol-tests
/tmp/x7-protocol-tests
```

## Inputs for a full build

- A Linux build host with Nix and Git, Python 3.12 or newer, Rust 1.95 with
  Cargo, OpenSSH and gzip. Nix cross-compiles the components for the DM870.
  The first build needs network access for upstream source and dependencies.
- A local **zd1** Git checkout containing commit
  `1cd511c0d37a758d6267675664bf78abd36a8ee6`. Prepare reads the pinned
  commit regardless of the branch or working tree. [Upstream revision](https://github.com/pawlowskialex/zd1/tree/1cd511c0d37a758d6267675664bf78abd36a8ee6)
- Your lawfully obtained Sony SRS-X7 `Laphroaigv2.09a.bcd` for stock version
  2.13.2.09. Its SHA-256 is
  `2f8af043a5058bc368ec99222ca391770ac75aca486719281e71cef03d12e129`.
  The packager checks this exact input and reproduces the original file
  byte for byte before assembling a replacement. [Sony SRS-X7 support](https://www.sony.ca/en/electronics/support/speakers-wireless-speakers/srs-x7)
- Your own authorized Nokia HD-1 files in a directory containing `kernel.bin`
  and `rootfs/`. The kernel is Nokia’s Linux 2.6.24.7 `Kernel_A` image,
  SHA-256 `66763b0011bed2c9e21282d5e8ed24e1c048d3b13aa062b76a9dfe854e3d2eff`.
  Keep the original module and radio-file layout below `rootfs/lib/`.
  The exact file list is under `[vendor]` in the prepared `module/os/os.toml`.
- An Ed25519 SSH **public** key and the region for your speaker. Supported
  firmware region codes are `JP`, `US` and `EU`. Choose `wired` for Ethernet.
  Wi-Fi streaming is unverified.

## Build on Linux

Use a new private working directory outside the source kit. Set these four
paths to your local files. Keep owner credentials, vendor files and build
outputs outside the prepared source tree. Nix copies that tree into its store.

```sh
KIT="$PWD"
BUILD="$HOME/x7-build"
UPSTREAM="$HOME/src/zd1"
STOCK="$HOME/firmware-inputs/Laphroaigv2.09a.bcd"
VENDOR="$HOME/firmware-inputs/nokia-hd1"

mkdir -m 700 "$BUILD"
python3 "$KIT/kit.py" prepare --upstream "$UPSTREAM" --stock "$STOCK" --output "$BUILD/src"
python3 "$KIT/kit.py" parts --tree "$BUILD/src" --output "$BUILD/parts"
cargo build --release --locked --manifest-path "$BUILD/src/x7/Cargo.toml" \
  --target-dir "$BUILD/host" -p x7-image
```

Prepare reads only the fixed register defaults used by the controller service
from the verified stock image. The parts step selects the production userspace,
the rescue userspace, the ARM boot stub, the Sony management service, the SPI
driver and their dependencies. It can take a while on a cold Nix store.

## Provision and pack offline

Generate a dedicated key with `ssh-keygen`, or pass an existing Ed25519
public key to the owner command. In this example, replace `EU` with the
speaker's region. Only the public key goes into the image. The owner command
discards key comments, generates a random dashboard password and writes it to
`owner/web-password`. The SSH server creates its own host key on the speaker.

```sh
ssh-keygen -t ed25519 -C "" -f "$BUILD/speaker-key"
python3 "$KIT/kit.py" owner --public-key "$BUILD/speaker-key.pub" \
  --country EU --interface wired --output "$BUILD/owner"
python3 "$KIT/kit.py" pack --image-tool "$BUILD/host/release/x7-image" \
  --tree "$BUILD/src" --parts "$BUILD/parts" --vendor "$VENDOR" \
  --stock "$STOCK" --owner "$BUILD/owner" --output "$BUILD/image"
```

The finished directory contains `sony-x7.bcd`, `system.cpio.gz`,
`recovery.cpio.gz` and `SHA256SUMS`. Pack verifies the kernel and rescue
inside the update, the system and its digest inside rescue, and the owner
credentials and their file modes in both archives. Recheck the directory
with `python3 "$KIT/verify.py" "$BUILD/image" "$BUILD/owner"`.

Keep the resulting files private. They now contain your management credentials
and vendor software. On an already provisioned speaker, existing management
credentials in persistent storage take precedence over the image defaults.

## Rights and provenance

The X7 source is provided under the enclosed MIT notice, with the Rust SPI
kernel module (`port/x7/kernel` crate) under GPL-2.0-only. The upstream MIT
notice accompanies the **zd1** adaptations. The image tools require separate
vendor inputs and build several dependencies with their own licenses. The
source kit grants no redistribution rights to those inputs. [Project MIT license](LICENSE)
· [Upstream MIT license](LICENSES/zd1.txt) · [GPL version 2](LICENSES/GPL-2.0-only.txt)
· [Upstream component notices](https://github.com/pawlowskialex/zd1/blob/1cd511c0d37a758d6267675664bf78abd36a8ee6/NOTICE.md)
· [Sharing and licensing](https://davelindo.github.io/srs-revival/sharing.html)
