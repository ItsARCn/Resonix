//! Google Cast stub (Spotify Cast Connect is not used with Resonix).

use std::net::IpAddr;

use anyhow::{Result, bail};
use md5::{Digest, Md5};

pub(crate) const SERVICE: &str = "_googlecast._tcp.local.";

/// Deterministic device id derived from the speaker's friendly name.
pub fn device_id(friendly_name: &str) -> String {
    format!("{:x}", Md5::digest(friendly_name))
}

pub fn start(
    _address: IpAddr,
    _port: u16,
    _name: &str,
    _exchange: impl FnMut(&str, &str) -> Result<String>,
) -> Result<()> {
    bail!("Cast receiver handoff is not supported in Resonix YouTube mode")
}
