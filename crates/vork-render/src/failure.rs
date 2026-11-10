//! Stable CLI failure categories carried as typed anyhow context.
#[derive(Clone, Copy, Debug)]
pub enum FailureKind {
    Usage,
    Scene,
    /// GPU work failed: device loss, allocation, a device or renderer limit.
    Device,
    /// A GPU, encoder, denoiser or Slang toolchain could not be set up; `vork setup` reports them.
    Setup,
}
impl std::fmt::Display for FailureKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Usage => "invalid command usage",
            Self::Scene => "invalid scene",
            Self::Device => "device or encoder failure",
            Self::Setup => "setup failure",
        })
    }
}
impl std::error::Error for FailureKind {}

/// Wrap operations that also evaluate scene inputs, retaining their precise category.
/// Leaf GPU failures can attach Device directly.
pub(crate) fn device(error: anyhow::Error) -> anyhow::Error {
    categorize(error, FailureKind::Device)
}
/// Wrap creating the GPU device, an encoder or the denoiser, retaining a precise category.
pub(crate) fn setup(error: anyhow::Error) -> anyhow::Error {
    categorize(error, FailureKind::Setup)
}
fn categorize(error: anyhow::Error, default: FailureKind) -> anyhow::Error {
    if error.downcast_ref::<FailureKind>().is_some() { return error; }
    let category = kind(&error).unwrap_or(default);
    error.context(category)
}
/// An attached category, else a cause whose category does not depend on where
/// it surfaces: a missing or wrong Slang compiler is a setup failure (a lazily
/// compiled variant meets it mid-render), and an encoder's device limit is a
/// device failure of a working device, though met while opening the encoder.
/// The causes are for crates below vork-render; vork-render code attaches a
/// FailureKind where it knows one.
pub(crate) fn kind(error: &anyhow::Error) -> Option<FailureKind> {
    error.downcast_ref::<FailureKind>().copied()
        .or_else(|| error.downcast_ref::<vork_slang::CompilerUnavailable>().map(|_| FailureKind::Setup))
        .or_else(|| error.downcast_ref::<vork_encode::DeviceLimit>().map(|_| FailureKind::Device))
}
/// A scene the renderer refuses (in its settings), as opposed to a device or setup failure.
pub(crate) fn is_scene(error: &anyhow::Error) -> bool {
    matches!(kind(error), Some(FailureKind::Scene))
}
/// Only setup failures point at `vork setup`; a failed render on a working
/// device (a scene beyond a renderer limit, say) is not one.
pub fn suggests_setup(error: &anyhow::Error) -> bool {
    matches!(kind(error), Some(FailureKind::Setup))
}

/// Categories survive contextual wrapping; filesystem errors retain their source.
pub fn exit_code(error: &anyhow::Error) -> u8 {
    if let Some(kind) = kind(error) {
        return match kind { FailureKind::Usage => 2, FailureKind::Scene => 3, FailureKind::Device | FailureKind::Setup => 4 };
    }
    if error.chain().any(|cause| cause.is::<std::io::Error>()) { 5 } else { 3 }
}

/// Run upstream parser code on user files. A panic there
/// is a bug in that crate for malformed input; return it as an error naming the
/// crate source line instead of unwinding through the renderer. Contained panics
/// are not printed; every other panic reaches the previous hook unchanged.
pub(crate) fn contain<T>(what: &str, run: impl FnOnce() -> anyhow::Result<T>) -> anyhow::Result<T> {
    use std::cell::{Cell, RefCell};
    thread_local! {
        static DEPTH: Cell<u32> = const { Cell::new(0) };
        static PANIC: RefCell<Option<String>> = const { RefCell::new(None) };
    }
    static HOOK: std::sync::Once = std::sync::Once::new();
    HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if DEPTH.get() == 0 {
                return previous(info);
            }
            let at = info.location().map_or_else(String::new, |at| {
                // Name the crate file, not the registry checkout path.
                let file = at.file().replace('\\', "/");
                let file = file.split_once("/registry/src/")
                    .and_then(|(_, rest)| rest.split_once('/'))
                    .map_or(file.as_str(), |(_, file)| file);
                format!(" at {file}:{}", at.line())
            });
            let message = info.payload_as_str().unwrap_or("panic");
            PANIC.set(Some(format!("{at}: {message}")));
        }));
    });
    DEPTH.set(DEPTH.get() + 1);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run));
    DEPTH.set(DEPTH.get() - 1);
    result.unwrap_or_else(|_| {
        let panic = PANIC.take().unwrap_or_default();
        Err(anyhow::anyhow!("{what} failed on malformed input (upstream panic{panic})"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn contained_panics_become_errors_with_their_location() {
        let error = contain("parser", || -> anyhow::Result<()> { panic!("bad index") }).unwrap_err();
        let text = format!("{error:#}");
        assert!(text.starts_with("parser failed on malformed input (upstream panic at "), "{text}");
        assert!(text.contains("failure.rs:") && text.ends_with(": bad index)"), "{text}");
        assert_eq!(contain("parser", || Ok(7)).unwrap(), 7);
    }
    #[test]
    fn categories_survive_context_without_matching_error_text() {
        assert_eq!(exit_code(&anyhow::anyhow!("bad field").context(FailureKind::Scene).context("load")), 3);
        assert_eq!(exit_code(&device(anyhow::anyhow!("adapter"))), 4);
        assert_eq!(exit_code(&device(anyhow::anyhow!("focus").context(FailureKind::Scene))), 3);
        assert_eq!(exit_code(&anyhow::anyhow!("destination").context(FailureKind::Usage)), 2);
        assert_eq!(exit_code(&anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::PermissionDenied)).context("write")), 5);
    }
    #[test]
    fn only_setup_failures_suggest_setup() {
        // A render beyond a renderer limit on a working device (here the path
        // occupancy backstop, an untagged frame error) is a device failure.
        let limit = device(anyhow::anyhow!("more glass solids overlap at a point").context("frame 0"));
        assert_eq!(exit_code(&limit), 4);
        assert!(!suggests_setup(&limit));
        let adapter = setup(anyhow::anyhow!("no adapter").context("create device"));
        assert_eq!(exit_code(&adapter), 4);
        assert!(suggests_setup(&adapter) && suggests_setup(&adapter.context("render")));
        // The first category attached is kept.
        assert!(!suggests_setup(&setup(anyhow::anyhow!("lost").context(FailureKind::Device))));
        assert!(!suggests_setup(&setup(anyhow::anyhow!("bad").context(FailureKind::Scene))));
    }
    #[test]
    fn slang_is_setup_and_encoder_limits_are_device_wherever_they_surface() {
        let slang = || anyhow::Error::new(vork_slang::CompilerUnavailable("run Slang compiler".into())).context("compile path variant");
        for error in [slang(), device(slang())] {
            assert_eq!(exit_code(&error), 4);
            assert!(suggests_setup(&error), "{error:#}");
        }
        assert!(matches!(device(slang()).downcast_ref::<FailureKind>(), Some(FailureKind::Setup)));
        let limit = || anyhow::Error::from(vork_encode::DeviceLimit("dimensions exceed device limits")).context("open encoder");
        for error in [limit(), setup(limit())] {
            assert_eq!(exit_code(&error), 4);
            assert!(!suggests_setup(&error), "{error:#}");
        }
    }
}
