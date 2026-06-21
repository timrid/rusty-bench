//! Scan subcommand — list reachable devices.

use std::io;

use futures::executor::block_on;
use rb_core::DriverRegistry;
use serde::Serialize;

use crate::util::write_json;

/// Scans all registered drivers and prints one line per reachable device.
///
/// # Errors
/// Propagates any driver scan error.
pub fn run_scan(writer: &mut dyn io::Write, json: bool) -> anyhow::Result<()> {
    let registry = DriverRegistry::with_default_factories();
    let results = block_on(registry.scan_all())?;

    if json {
        let scan_results: Vec<ScanEntry> = results
            .iter()
            .map(|r| ScanEntry {
                driver: r.driver.clone(),
                vendor: r.candidate.info.vendor.clone(),
                model: r.candidate.info.model.clone(),
                address: r.candidate.address.clone(),
            })
            .collect();
        return write_json(&scan_results, writer);
    }

    if results.is_empty() {
        writeln!(writer, "No devices found.")?;
    } else {
        for r in &results {
            writeln!(
                writer,
                "[{}] {}/{} @ {}",
                r.driver, r.candidate.info.vendor, r.candidate.info.model, r.candidate.address
            )?;
        }
    }
    Ok(())
}

#[derive(Serialize)]
struct ScanEntry {
    driver: String,
    vendor: String,
    model: String,
    address: String,
}
