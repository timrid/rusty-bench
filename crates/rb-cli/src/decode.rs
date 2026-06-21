//! Decode subcommand — decode a previously recorded capture file.

use std::io;
use std::path::Path;

/// Decodes a previously recorded capture file.
///
/// # Errors
/// Returns an error if the input file cannot be read or the format is not
/// supported. The decode infrastructure is not yet fully implemented.
pub fn run_decode(
    _input_file: &Path,
    _input_format: Option<&str>,
    _decoders: Option<&str>,
    _output_format: &str,
    writer: &mut dyn io::Write,
) -> anyhow::Result<()> {
    writeln!(
        writer,
        "Decode is not yet implemented. Use 'rusty-bench record' to capture data first."
    )?;
    Ok(())
}
