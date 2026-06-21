//! Shared internal helpers used by multiple command modules.

use std::io;

use anyhow::Context as _;
use futures::executor::block_on;
use rb_core::DriverRegistry;
use rb_device::Device;
use serde::Serialize;

/// Write `value` as pretty-printed JSON to `writer`, followed by a newline.
pub fn write_json(value: &impl Serialize, writer: &mut dyn io::Write) -> anyhow::Result<()> {
    let json = serde_json::to_string_pretty(value)?;
    writeln!(writer, "{json}")?;
    Ok(())
}

/// Scans for `address` across all registered drivers and connects to it.
pub fn find_and_connect(address: &str) -> anyhow::Result<Box<dyn Device>> {
    let registry = DriverRegistry::with_default_factories();
    let results = block_on(registry.scan_all())?;
    let result = results
        .into_iter()
        .find(|r| r.candidate.address == address)
        .with_context(|| format!("no device found at address: {address}"))?;
    let device = block_on(registry.connect(&result.driver, &result.candidate))?;
    Ok(device)
}
