//! RustyBench command-line interface — a headless, scriptable single-shot tool.

use clap::Parser;

/// RustyBench CLI.
#[derive(Parser, Debug)]
#[command(name = "rusty-bench", version, about)]
struct Cli {}

fn main() -> anyhow::Result<()> {
    let _cli = Cli::parse();
    let registry = rb_core::DriverRegistry::with_default_factories();
    println!(
        "RustyBench CLI — {} driver(s) available (M3 skeleton)",
        registry.len()
    );
    Ok(())
}
