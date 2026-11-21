//! Assemble Sony roots from the existing pinned components. Binary archive and
//! payload generators are retained unchanged; container wrapping is Sony-aware.
use crate::{Result, bcd};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
use toml::Value;

// Sony's network updater and certificate store extend through 49 MiB on this unit.
// Keep the full lower half read-only to Linux, including space for bad blocks.
const KERNEL_CMDLINE: &str = concat!(
    "console=ttyS1,115200 mem=31M ",
    "mtdparts=sulu-nand:64m(vendor)ro,24m(system-a),24m(system-b),16m(data) panic=10",
);

fn run(command: &mut Command) -> Result<()> {
    println!("image: {command:?}");
    if !command.status()?.success() {
        return Err("image tool failed".into());
    }
    Ok(())
}

fn copy(source: &Path, target: &Path) -> Result<()> {
    let kind = fs::symlink_metadata(source)?.file_type();
    if kind.is_dir() {
        fs::create_dir_all(target)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy(&entry.path(), &target.join(entry.file_name()))?;
        }
    } else {
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        if fs::symlink_metadata(target).is_ok() {
            fs::remove_file(target)?;
        }
        if kind.is_symlink() {
            symlink(fs::read_link(source)?, target)?;
        } else if kind.is_file() {
            fs::copy(source, target)?;
        } else {
            return Err("source tree contains a special file".into());
        }
    }
    Ok(())
}
fn text(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    Ok(())
}

fn copy_management_credentials(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)?;
    for name in ["authorized_keys", "web-password"] {
        let input = source.join(name);
        if !fs::symlink_metadata(&input)?.is_file() {
            return Err(format!("expected a regular owner file: {name}").into());
        }
        let target = destination.join(name);
        fs::copy(input, &target)?;
        fs::set_permissions(target, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn array(value: &Value) -> Result<Vec<&str>> {
    value
        .as_array()
        .ok_or("expected configuration array")?
        .iter()
        .map(|v| v.as_str().ok_or_else(|| "expected configuration string".into()))
        .collect()
}
fn string(value: &Value) -> Result<&str> {
    value.as_str().ok_or_else(|| "expected configuration string".into())
}
fn links(root: &Path, names: &Value, target: &str) -> Result<()> {
    for name in array(names)? {
        symlink(target, root.join("bin").join(name))?;
    }
    Ok(())
}
fn archive(repo: &Path, root: &Path, output: &Path, slot: bool) -> Result<()> {
    let mut command = Command::new("python3");
    command.arg("-I").arg(repo.join("module/os/mkcpio.py"));
    if slot {
        command.arg("--slot");
    }
    run(command.arg(root).arg(output))?;
    let compressed = output.with_extension("cpio.gz");
    let file = fs::OpenOptions::new().write(true).create_new(true).open(compressed)?;
    run(Command::new("gzip").args(["-n", "-9", "-c"]).arg(output).stdout(Stdio::from(file)))
}
fn payload(
    repo: &Path,
    parts: &Path,
    linux_kernel: &Path,
    archive: &Path,
    output: &Path,
) -> Result<()> {
    let mut command = Command::new("python3");
    command
        .arg("-I")
        .arg(repo.join("module/os/mkpayload.py"))
        .arg(parts.join("boot/linuxboot.bin"))
        .arg(linux_kernel)
        .arg(archive)
        .arg(output)
        .arg("--deflate-kernel")
        .arg("--initrd-at")
        .arg("0x61000000");
    run(command.arg("--slots").arg("--cmdline").arg(KERNEL_CMDLINE))
}

pub fn assemble(
    repo: &Path,
    parts: &Path,
    vendor: &Path,
    template: &Path,
    owner: &Path,
    output: &Path,
) -> Result<()> {
    let stock = fs::read(template)?;
    bcd::verify_stock(&stock)?;
    let cfg: Value = toml::from_str(&fs::read_to_string(repo.join("module/os/os.toml"))?)?;
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new().mode(0o700).create(output)?;
    let management = parts.join("x7-management/bin/x7-management");
    let spi_module = parts.join("x7-kernel/lib/x7-spi.ko");
    let linux_kernel = output.join("kernel-rx.bin");
    run(Command::new("python3")
        .arg("-I")
        .arg(repo.join("x7/kernel/patch-ethernet-rx.py"))
        .arg(vendor.join("kernel.bin"))
        .arg(&linux_kernel))?;
    let system = output.join("system-root");
    copy(&fs::canonicalize(parts.join("busybox"))?, &system)?;
    for directory in [
        "dev",
        "proc",
        "sys",
        "tmp",
        "var",
        "mnt",
        "root",
        "run",
        "data",
        "etc",
        "lib",
        "usr/bin",
        "usr/sbin",
        "usr/share/zd1",
        "usr/share/x7",
        "usr/lib",
    ] {
        fs::create_dir_all(system.join(directory))?;
    }
    let vendor = vendor.join("rootfs");
    for directory in ["modules", "firmware"] {
        symlink(format!("/usr/lib/{directory}"), system.join("lib").join(directory))?;
    }
    for module in array(&cfg["vendor"]["modules"])? {
        let path = PathBuf::from(format!("lib/modules/2.6.24.7/kernel/{module}.ko"));
        copy(&vendor.join(&path), &system.join("usr").join(&path))?;
    }
    for file in array(&cfg["vendor"]["firmware"])? {
        copy(&vendor.join(file), &system.join("usr").join(file))?;
    }
    for (source, target) in
        [("alsa-lib/share/alsa", "usr/share/alsa"), ("alsa-utils/bin", "usr/bin"), ("wpa_supplicant/bin", "usr/sbin")]
    {
        copy(&parts.join(source), &system.join(target))?;
    }
    copy(&parts.join("apps/bin/zd1"), &system.join("usr/bin/zd1"))?;
    links(&system, &cfg["apps"]["core"], "../usr/bin/zd1")?;
    links(&system.join("usr"), &cfg["apps"]["bins"], "zd1")?;
    copy(&repo.join(string(&cfg["image"]["overlay"])?), &system)?;
    copy(&management, &system.join("usr/bin/x7-management"))?;
    let destination = system.join("usr/share/x7/provisioning");
    copy_management_credentials(owner, &destination)?;
    for name in ["CountryCode", "NetIFMode"] {
        let path = owner.join(name);
        if !fs::symlink_metadata(&path)?.is_file() {
            return Err(format!("expected a regular owner file: {name}").into());
        }
        fs::copy(path, destination.join(name))?;
    }
    copy(&spi_module, &system.join("usr/lib/x7-spi.ko"))?;
    let modules = array(&cfg["vendor"]["load"])?.join("\n") + "\n";
    text(&system.join("etc/modules"), &modules)?;
    text(
        &system.join("usr/lib/os-release"),
        "NAME=\"Sony SRS-X7 replacement\"\nID=sony-x7\nVERSION_ID=0.1.0\n",
    )?;
    for file in ["os-release", "resolv.conf"] {
        symlink(format!("/run/{file}"), system.join("etc").join(file))?;
    }
    for program in ["usr/bin/zd1", "usr/bin/x7-management"] {
        fs::set_permissions(system.join(program), fs::Permissions::from_mode(0o755))?;
    }
    let system_archive = output.join("system.cpio");
    archive(repo, &system, &system_archive, true)?;
    let system_compressed = output.join("system.cpio.gz");
    let system_id = bcd::sha256(&fs::read(&system_compressed)?);
    text(&output.join("system-id"), &format!("{system_id}\n"))?;

    let rescue = output.join("recovery-root");
    copy(&fs::canonicalize(parts.join("busybox-rescue"))?, &rescue)?;
    for directory in
        ["dev", "proc", "sys", "tmp", "var", "run", "data", "mnt", "etc", "usr/bin", "usr/lib", "usr/share/x7"]
    {
        fs::create_dir_all(rescue.join(directory))?;
    }
    copy(&parts.join("apps-core/bin/zd1"), &rescue.join("bin/zd1"))?;
    links(&rescue, &cfg["apps"]["rescue"], "zd1")?;
    symlink("bin/zd1", rescue.join("init"))?;
    copy(&management, &rescue.join("usr/bin/x7-management"))?;
    copy_management_credentials(owner, &rescue.join("usr/share/x7/provisioning"))?;
    copy(&spi_module, &rescue.join("usr/lib/x7-spi.ko"))?;
    copy(&system_compressed, &rescue.join("usr/share/x7/system.cpio.gz"))?;
    text(&rescue.join("usr/share/x7/system-id"), &format!("{system_id}\n"))?;
    text(
        &rescue.join("etc/os-release"),
        "NAME=\"Sony SRS-X7 recovery\"\nID=sony-x7\nVERSION_ID=0.1.0\nVARIANT_ID=rescue\n",
    )?;
    let recovery_archive = output.join("recovery.cpio");
    archive(repo, &rescue, &recovery_archive, false)?;
    let boot_payload = output.join("payload.bin");
    payload(repo, parts, &linux_kernel, &output.join("recovery.cpio.gz"), &boot_payload)?;
    let wrapped = bcd::build(&stock, &fs::read(&boot_payload)?)?;
    fs::write(output.join("sony-x7.bcd"), &wrapped)?;

    text(
        &output.join("SHA256SUMS"),
        &[
            format!("{}  sony-x7.bcd\n", bcd::sha256(&wrapped)),
            format!("{system_id}  system.cpio.gz\n"),
            format!("{}  recovery.cpio.gz\n", bcd::sha256(&fs::read(output.join("recovery.cpio.gz"))?)),
        ]
        .concat(),
    )?;
    println!(
        "image: Sony bundle {} bytes; system {} bytes compressed; review required, no device touched",
        wrapped.len(),
        fs::metadata(&system_compressed)?.len()
    );
    Ok(())
}
