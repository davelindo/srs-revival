//! Sony's container prefix is longer than the reference project's DMP tag.
//! Preserve everything up to the first section offset, including Generic.
use crate::Result;
use sha2::{Digest, Sha256};

const HEADER: usize = 0xb0;
const STAGE2: usize = 0x2000;
const PART_HEADER: usize = 0x80;
pub const SONY_SHA256: &str = "2f8af043a5058bc368ec99222ca391770ac75aca486719281e71cef03d12e129";

fn word(data: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(data.get(at..at + 4).ok_or("truncated BCD word")?.try_into()?))
}
fn put(data: &mut [u8], at: usize, value: u32) {
    data[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
pub fn sha256(data: &[u8]) -> String {
    use std::fmt::Write;
    let mut result = String::with_capacity(64);
    for byte in Sha256::digest(data) {
        write!(result, "{byte:02x}").unwrap();
    }
    result
}
fn header_crc(data: &[u8]) -> u32 {
    let mut header = data[..HEADER].to_vec();
    put(&mut header, 0x2c, 0);
    crc32fast::hash(&header)
}
fn fletcher(data: &[u8]) -> u32 {
    let (mut a, mut b) = (0xffffu32, 0xffffu32);
    for group in data.chunks(720) {
        for pair in group.chunks(2) {
            a += u16::from_le_bytes([pair[0], pair.get(1).copied().unwrap_or(0)]) as u32;
            b = b.wrapping_add(a);
        }
        a = (a & 0xffff) + (a >> 16);
        b = (b & 0xffff) + (b >> 16);
    }
    a = (a & 0xffff) + (a >> 16);
    b = (b & 0xffff) + (b >> 16);
    a | (b << 16)
}

#[derive(Clone)]
struct Section<'a> {
    at: usize,
    nand: u32,
    capacity: usize,
    data: &'a [u8],
}
fn sections(data: &[u8]) -> Result<Vec<Section<'_>>> {
    if data.len() < HEADER || &data[..4] != b"bCoD" || header_crc(data) != word(data, 0x2c)? {
        return Err("BCD header CRC/magic mismatch".into());
    }
    let mut sections = Vec::new();
    let mut end = word(data, 0x30)? as usize;
    if !(HEADER..=4096).contains(&end) {
        return Err("unexpected BCD prefix length".into());
    }
    for index in 0..3 {
        let at = 0x30 + index * 0x20;
        let offset = word(data, at)? as usize;
        let length = word(data, at + 8)? as usize;
        let capacity = word(data, at + 16)? as usize;
        if offset != end || length == 0 || length.checked_add(PART_HEADER).is_none_or(|n| n > capacity) {
            return Err("invalid BCD section bounds".into());
        }
        end = offset.checked_add(length).ok_or("BCD section overflow")?;
        let section = data.get(offset..end).ok_or("truncated BCD section")?;
        if crc32fast::hash(section) != word(data, at + 12)? {
            return Err("BCD section CRC mismatch".into());
        }
        sections.push(Section { at, nand: word(data, at + 4)?, capacity, data: section });
    }
    if data[end..].iter().any(|&byte| byte != 0xff) {
        return Err("unexpected BCD trailer".into());
    }
    Ok(sections)
}

pub fn build(template: &[u8], payload: &[u8]) -> Result<Vec<u8>> {
    let sections = sections(template)?;
    let app = &sections[0];
    if app.nand != 0x240000 || app.capacity != 0xa00000 || app.data.len() < STAGE2 || word(app.data, 0)? != 0xaaaa5555 {
        return Err("not the verified SRS-X7 application layout".into());
    }
    let mut payload = payload.to_vec();
    payload.resize(payload.len().next_multiple_of(4), 0);
    if payload.is_empty() || payload.len() + STAGE2 + PART_HEADER > app.capacity {
        return Err("Sony payload exceeds app partition".into());
    }
    let mut replacement = app.data[..STAGE2].to_vec();
    replacement.extend_from_slice(&payload);
    put(&mut replacement, 0x24, app.nand + PART_HEADER as u32 + STAGE2 as u32);
    put(&mut replacement, 0x28, payload.len() as u32);
    put(&mut replacement, 0x2c, fletcher(&payload));
    let crc_end = 0x18usize.checked_add(word(&replacement, 0x10)? as usize * 8).ok_or("preloader length overflow")?;
    if crc_end > STAGE2 {
        return Err("preloader overlaps stage 2".into());
    }
    let crc = crc32fast::hash(&replacement[0x18..crc_end]);
    put(&mut replacement, 0x14, crc);
    let prefix = word(template, 0x30)? as usize;
    let mut output = template[..prefix].to_vec();
    for (index, section) in sections.iter().enumerate() {
        let data = if index == 0 { replacement.as_slice() } else { section.data };
        let offset = output.len() as u32;
        put(&mut output, section.at, offset);
        put(&mut output, section.at + 8, data.len() as u32);
        put(&mut output, section.at + 12, crc32fast::hash(data));
        output.extend_from_slice(data);
    }
    let crc = header_crc(&output);
    put(&mut output, 0x2c, crc);
    output.resize(output.len().next_multiple_of(1024), 0xff);
    let checked = self::sections(&output)?;
    if checked[1].data != sections[1].data || checked[2].data != sections[2].data {
        return Err("factory configuration or resource section changed".into());
    }
    Ok(output)
}

pub fn verify_stock(template: &[u8]) -> Result<()> {
    if sha256(template) != SONY_SHA256 {
        return Err("official Sony image SHA-256 mismatch".into());
    }
    let app = &sections(template)?[0];
    let payload = app.data.get(STAGE2..).ok_or("missing stock stage 2")?;
    if word(app.data, 0x28)? as usize != payload.len() || word(app.data, 0x2c)? != fletcher(payload) {
        return Err("stock stage-2 checksum mismatch".into());
    }
    if build(template, payload)? != template {
        return Err("stock BCD round trip is not byte-identical".into());
    }
    Ok(())
}
