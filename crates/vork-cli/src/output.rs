use std::io::Write;

#[derive(Debug)]
struct Stdout;
impl std::fmt::Display for Stdout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("write stdout") }
}
impl std::error::Error for Stdout {}

pub fn context(error: anyhow::Error) -> anyhow::Error {
    if error.downcast_ref::<std::io::Error>().is_some() { error.context(Stdout) } else { error }
}
pub fn error(error: std::io::Error) -> anyhow::Error { context(error.into()) }

pub fn line(args: std::fmt::Arguments<'_>) -> anyhow::Result<()> {
    writeln!(std::io::stdout().lock(), "{args}").map_err(error)
}

pub fn broken_pipe(error: &anyhow::Error) -> bool {
    error.downcast_ref::<Stdout>().is_some() && error.chain().any(|cause| cause.downcast_ref::<std::io::Error>()
        .is_some_and(|error| error.kind() == std::io::ErrorKind::BrokenPipe))
}

#[cfg(test)]
mod tests {
    #[test]
    fn tool_and_usage_failures_are_not_stdout_failures() {
        for message in ["setup tools failed: Slang", "unknown help topic"] {
            let error = super::context(anyhow::anyhow!("{message}"));
            assert_eq!(format!("{error:#}"), message);
            assert!(!super::broken_pipe(&error));
        }
    }
    #[test]
    fn only_stdout_broken_pipes_are_clean_termination() {
        let pipe = || std::io::Error::from(std::io::ErrorKind::BrokenPipe);
        assert!(super::broken_pipe(&super::error(pipe())));
        assert!(!super::broken_pipe(&pipe().into()));
        assert!(!super::broken_pipe(&super::error(std::io::Error::from(std::io::ErrorKind::StorageFull))));
    }
}
