#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod power_link;
#[cfg(target_os = "linux")]
mod runtime;
#[cfg(any(target_os = "linux", test))]
mod startup;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod state;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod tandem;
#[cfg(target_os = "linux")]
pub use runtime::{load_driver, serve};

#[cfg(any(target_os = "linux", test))]
fn renderer_identity(mac: [u8; 6]) -> Vec<u8> {
    // KnOS 0x60167f54 derives Dev0 from this prefix and the factory MAC.
    // At startup 0x60164f2c announces the compact ID as NDB subtype zero.
    let [a, b, c, d, e, f] = mac;
    format!("\x005f9ec1b3ed5919004530{a:02x}{b:02x}{c:02x}{d:02x}{e:02x}{f:02x}\0").into_bytes()
}

#[cfg(test)]
mod tests {
    #[test]
    fn announces_compact_factory_renderer_id_for_the_unit_mac() {
        assert_eq!(super::renderer_identity([2, 0, 10, 176, 193, 255]), b"\x005f9ec1b3ed591900453002000ab0c1ff\0");
    }
}
