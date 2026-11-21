//! Sony-specific policy, separate from the reference host protocol's settings.
use crate::Error;
use std::{
    fs,
    path::{Path, PathBuf},
};

pub const REPORT: &str = "/run/x7-power";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Desired {
    pub network: bool,
    pub automatic: Option<bool>,
}
impl Default for Desired {
    fn default() -> Self {
        Self { network: true, automatic: None }
    }
}
fn path(settings: &str) -> PathBuf {
    Path::new(settings).parent().unwrap_or(Path::new("/data/zd1")).join("power")
}
pub fn load(settings: &str) -> Desired {
    let mut desired = Desired::default();
    for line in fs::read_to_string(path(settings)).unwrap_or_default().lines() {
        match line {
            "network=off" => desired.network = false,
            "network=on" => desired.network = true,
            "automatic=on" => desired.automatic = Some(true),
            "automatic=off" => desired.automatic = Some(false),
            _ => {}
        }
    }
    desired
}
pub fn save(settings: &str, network: &str, automatic: &str) -> Result<(), Error> {
    if !matches!(network, "on" | "off") || !matches!(automatic, "keep" | "on" | "off") {
        return Err("Choose valid standby settings.".into());
    }
    zd1_sys::write_private(
        path(settings).to_str().ok_or("invalid power settings path")?,
        &format!("network={network}\nautomatic={automatic}\n"),
    )?;
    Ok(())
}
