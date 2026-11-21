use std::io;

const COUNTRY: u8 = 0x81;
const INTERFACE: u8 = 0x87;

pub(super) struct Settings {
    country: Option<u8>,
    interface: Option<u8>,
}

impl Settings {
    #[cfg(target_os = "linux")]
    pub(super) fn load() -> Self {
        Self {
            country: std::fs::read_to_string("/usr/share/x7/provisioning/CountryCode")
                .ok()
                .and_then(|name| country_code(&name)),
            interface: std::fs::read_to_string("/usr/share/x7/provisioning/NetIFMode")
                .ok()
                .and_then(|name| interface_mode(&name)),
        }
    }

    pub(super) fn initial_value(&self, register: u8, default: [u8; 2]) -> [u8; 2] {
        match register {
            COUNTRY => self.country.map_or(default, |region| [region, 0]),
            INTERFACE => self.interface.map_or(default, |mode| [mode, 0]),
            _ => default,
        }
    }

    pub(super) fn fixed_reply(&self, register: u8, body: &[u8]) -> Option<(&'static str, [u8; 2])> {
        match register {
            COUNTRY => country_reply(self.country, body).map(|reply| ("Sony region reply 81", reply)),
            INTERFACE => interface_reply(self.interface, body).map(|reply| ("Sony interface reply 87", reply)),
            _ => None,
        }
    }
}

fn country_code(name: &str) -> Option<u8> {
    match name.trim() {
        "JP" => Some(0),
        "US" => Some(1),
        "EU" => Some(2),
        _ => None,
    }
}

/// KnOS 0x602592f8 reports its configured region for command 3, and
/// acknowledges a selection of that region with the same two-byte notification.
fn country_reply(country: Option<u8>, body: &[u8]) -> Option<[u8; 2]> {
    let country = country?;
    match body {
        [command, 0] if *command == 3 || *command == country => Some([country, 0]),
        _ => None,
    }
}

fn interface_mode(name: &str) -> Option<u8> {
    match name.trim() {
        "autosense" => Some(0),
        "wired" => Some(1),
        "wireless" => Some(2),
        _ => None,
    }
}

/// KnOS 0x602599e8 publishes the configured interface for query 03 00.
fn interface_reply(mode: Option<u8>, body: &[u8]) -> Option<[u8; 2]> {
    match (mode, body) {
        (Some(value), [3, 0]) => Some([value, 0]),
        _ => None,
    }
}

/// Replies to the Armada's startup metadata on variable register 0x7e.
/// KnOS 0x602560d8 returns a separate application response after the SPI ACK.
pub(crate) fn reply(body: &[u8], store: impl FnOnce(&str, &str) -> io::Result<()>) -> Option<Vec<u8>> {
    match body {
        // The replacement has no Sony AutoMagic distribution to offer.
        [1, 0] => Some(vec![1, 0xe0]),
        [2, 0] => Some(vec![2, 0xe0, 0]),
        [7, group, fields @ ..] if fields.len() == 128 => {
            let (key, value) = fields.split_at(64);
            let field = |bytes: &[u8]| bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
            let key = &key[..field(key)];
            let name = match (group, key) {
                (1, b"Model") => Some("sony-model"),
                (4, b"MC_Version") => Some("sony-controller-version"),
                _ => None,
            };
            let saved = name
                .zip(std::str::from_utf8(&value[..field(value)]).ok())
                .is_some_and(|(name, value)| store(name, value).is_ok());
            let mut response = vec![0; 67];
            response[..2].copy_from_slice(&body[..2]);
            response[2..2 + key.len()].copy_from_slice(key);
            response[66] = if saved { 1 } else { 0xe0 };
            Some(response)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interface_query_returns_the_configured_mode() {
        for (name, value) in [("autosense", 0), ("wired\n", 1), ("wireless", 2)] {
            let settings = Settings { country: None, interface: interface_mode(name) };
            assert_eq!(settings.initial_value(INTERFACE, [3, 0]), [value, 0]);
            assert_eq!(settings.fixed_reply(INTERFACE, &[3, 0]).map(|(_, reply)| reply), Some([value, 0]));
            assert_eq!(settings.fixed_reply(INTERFACE, &[1, 0]), None);
        }
        assert_eq!(interface_reply(interface_mode(""), &[3, 0]), None);
    }

    #[test]
    fn reports_factory_region_and_acknowledges_matching_selection() {
        for (name, value) in [("JP", 0), ("US\n", 1), ("EU", 2)] {
            let settings = Settings { country: country_code(name), interface: None };
            assert_eq!(settings.initial_value(COUNTRY, [3, 0]), [value, 0]);
            assert_eq!(settings.fixed_reply(COUNTRY, &[3, 0]).map(|(_, reply)| reply), Some([value, 0]));
            assert_eq!(settings.fixed_reply(COUNTRY, &[value, 0]).map(|(_, reply)| reply), Some([value, 0]));
        }
        assert_eq!(country_reply(country_code(""), &[3, 0]), None);
        assert_eq!(country_reply(country_code("US"), &[2, 0]), None);
    }

    #[test]
    fn controller_identity_is_acknowledged_after_storage() {
        let mut record = vec![0; 130];
        record[..7].copy_from_slice(b"\x07\x01Model");
        record[66..72].copy_from_slice(b"SRS-X7");
        let mut ack = record[..67].to_vec();
        ack[66] = 1;
        assert_eq!(
            reply(&record, |key, value| {
                assert_eq!((key, value), ("sony-model", "SRS-X7"));
                Ok(())
            }),
            Some(ack.clone())
        );
        ack[66] = 0xe0;
        assert_eq!(reply(&record, |_, _| Err(io::Error::from(io::ErrorKind::StorageFull))), Some(ack));
    }
}
