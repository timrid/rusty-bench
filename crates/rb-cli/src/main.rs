//! RustyBench command-line interface — a headless, scriptable single-shot tool.

use clap::Parser;

/// RustyBench CLI.
#[derive(Parser, Debug)]
#[command(name = "rusty-bench", version, about)]
struct Cli {}

fn main() -> anyhow::Result<()> {
    let _cli = Cli::parse();
    println!("RustyBench CLI — {} (M0 skeleton)", rb_core::CRATE);
    Ok(())
}
