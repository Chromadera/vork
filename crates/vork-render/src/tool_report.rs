//! Setup's subprocess diagnostics are opt-in; routine failures stay one line.
use anyhow::Result;
use std::io::Write;

pub(crate) fn encoder_probe(out: &mut impl Write, encoder: &str, result: Result<()>, verbose: bool) -> Result<()> {
    match result {
        Ok(()) => writeln!(out, "{encoder}: initialization ready")?,
        Err(error) => {
            writeln!(out, "{encoder}: initialization unavailable; readback MP4 uses libx264 if available. Check the driver and FFmpeg build; run vork setup --verbose for probe output.")?;
            if verbose { writeln!(out, "{encoder} probe output:\n{error:#}")?; }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn multiline_probe_output_requires_verbose() -> Result<()> {
        for verbose in [false, true] {
            let mut output = Vec::new();
            encoder_probe(&mut output, "FFmpeg h264_nvenc", Err(anyhow::anyhow!("driver API mismatch\nraw line two\nraw line three")), verbose)?;
            let output = String::from_utf8(output)?;
            assert_eq!(output.contains("raw line two"), verbose);
            if !verbose { assert_eq!(output.lines().count(), 1); }
        }
        let mut output = Vec::new();
        encoder_probe(&mut output, "FFmpeg h264_nvenc", Ok(()), false)?;
        assert_eq!(String::from_utf8(output)?, "FFmpeg h264_nvenc: initialization ready\n");
        Ok(())
    }
}
