use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{self, Read},
    path::PathBuf,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use subtle::ConstantTimeEq;

const LIMIT: usize = 8;
const LIFETIME: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const OFFLINE_LIFETIME: Duration = Duration::from_secs(12 * 60 * 60);
pub(super) const COOKIE: &str = "x7_session";

#[derive(Clone)]
enum Expires {
    At(u64),
    Offline(Instant),
}

#[derive(Clone)]
struct Entry {
    id: String,
    expires: Expires,
}

pub(super) struct Sessions {
    path: PathBuf,
    entries: Vec<Entry>,
}

fn epoch() -> Option<u64> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    // The speaker starts at 1970 before NTP. Wired recovery can remain offline.
    (now >= 1_577_836_800).then_some(now)
}

fn hex_digest(parts: &[&[u8]]) -> String {
    use std::fmt::Write as _;
    let mut hash = Sha256::new();
    for bytes in parts {
        hash.update(bytes);
    }
    let mut text = String::with_capacity(64);
    for byte in hash.finalize() {
        write!(text, "{byte:02x}").expect("writing to a String");
    }
    text
}

fn identifier(password: &[u8], token: &str) -> String {
    hex_digest(&[b"x7 session\0", password, b"\0", token.as_bytes()])
}

fn is_token(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|value| matches!(value, b'0'..=b'9' | b'a'..=b'f'))
}

pub(super) fn cookie(header: &[u8]) -> Option<&str> {
    let fields = std::str::from_utf8(header).ok()?.split(';');
    let mut tokens = fields.filter_map(|field| {
        let (name, value) = field.trim().split_once('=')?;
        (name == COOKIE).then_some(value)
    });
    let token = tokens.next()?;
    (is_token(token) && tokens.next().is_none()).then_some(token)
}

pub(super) fn csrf(token: &str) -> String {
    hex_digest(&[b"x7 form\0", token.as_bytes()])
}

impl Sessions {
    pub(super) fn load(path: PathBuf) -> io::Result<Self> {
        let mut text = String::new();
        match File::open(&path) {
            Ok(file) => {
                file.take(4096).read_to_string(&mut text)?;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let entries = text
            .lines()
            .filter_map(|line| {
                let (deadline, id) = line.split_once(' ')?;
                if !is_token(id) {
                    return None;
                }
                Some(Entry { id: id.to_owned(), expires: Expires::At(deadline.parse().ok()?) })
            })
            .take(LIMIT)
            .collect();
        Ok(Self { path, entries })
    }

    fn commit(&mut self, entries: Vec<Entry>) -> io::Result<()> {
        use std::fmt::Write as _;
        let mut persisted = String::new();
        for entry in &entries {
            if let Expires::At(deadline) = entry.expires {
                writeln!(persisted, "{deadline} {}", entry.id).expect("writing to a String");
            }
        }
        zd1_sys::write_private(
            self.path.to_str().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid session path"))?,
            &persisted,
        )?;
        self.entries = entries;
        Ok(())
    }

    pub(super) fn contains(&self, password: &[u8], token: &str) -> bool {
        let id = identifier(password, token);
        let now = epoch();
        self.entries.iter().any(|entry| {
            bool::from(entry.id.as_bytes().ct_eq(id.as_bytes()))
                && match entry.expires {
                    Expires::At(deadline) => now.is_some_and(|time| time < deadline),
                    Expires::Offline(deadline) => Instant::now() < deadline,
                }
        })
    }

    pub(super) fn issue(&mut self, password: &[u8], token: &str) -> io::Result<String> {
        let now = epoch();
        let mut entries: Vec<_> = self
            .entries
            .iter()
            .filter(|entry| match entry.expires {
                Expires::At(deadline) => now.is_none_or(|time| time < deadline),
                Expires::Offline(deadline) => Instant::now() < deadline,
            })
            .cloned()
            .collect();
        if entries.len() == LIMIT {
            entries.remove(0);
        }
        let expires = now.map_or_else(
            || Expires::Offline(Instant::now() + OFFLINE_LIFETIME),
            |time| Expires::At(time + LIFETIME.as_secs()),
        );
        entries.push(Entry { id: identifier(password, token), expires });
        self.commit(entries)?;
        let mut header = format!("Set-Cookie: {COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict");
        if now.is_some() {
            header.push_str(&format!("; Max-Age={}", LIFETIME.as_secs()));
        }
        Ok(header)
    }

    pub(super) fn revoke(&mut self, password: &[u8], token: &str) -> io::Result<()> {
        let id = identifier(password, token);
        let entries = self.entries.iter().filter(|entry| entry.id != id).cloned().collect();
        self.commit(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_cookie_expires_and_cannot_be_used_after_a_password_change() {
        let token = "a".repeat(64);
        let password = b"a speaker password";
        let mut sessions = Sessions {
            path: PathBuf::new(),
            entries: vec![Entry { id: identifier(password, &token), expires: Expires::At(epoch().unwrap() + 60) }],
        };
        assert!(sessions.contains(password, &token));
        assert!(!sessions.contains(b"a different speaker password", &token));
        sessions.entries[0].expires = Expires::At(epoch().unwrap() - 1);
        assert!(!sessions.contains(password, &token));
    }
}
