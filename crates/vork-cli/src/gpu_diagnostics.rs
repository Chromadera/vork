//! Best-effort Linux ICD diagnostics after GPU initialization fails. Never selects
//! a driver or changes renderer behavior.
#[cfg(not(target_os = "linux"))]
pub fn report() -> Option<String> {
    None
}

#[cfg(target_os = "linux")]
pub fn report() -> Option<String> {
    use std::{env, fs, path::PathBuf};
    let files: Vec<PathBuf> = if let Some(value) =
        env::var_os("VK_DRIVER_FILES").or_else(|| env::var_os("VK_ICD_FILENAMES"))
    {
        env::split_paths(&value).collect()
    } else {
        let mut dirs = vec![
            PathBuf::from("/etc/vulkan/icd.d"),
            PathBuf::from("/usr/share/vulkan/icd.d"),
            PathBuf::from("/usr/local/share/vulkan/icd.d"),
        ];
        if let Some(home) = env::var_os("HOME") {
            dirs.push(PathBuf::from(home).join(".local/share/vulkan/icd.d"));
        }
        dirs.into_iter()
            .flat_map(|dir| {
                fs::read_dir(dir)
                    .into_iter()
                    .flatten()
                    .filter_map(Result::ok)
                    .map(|entry| entry.path())
            })
            .collect()
    };
    let mut problems = Vec::new();
    for path in files {
        if let Some(problem) = inspect(&path) {
            problems.push(format!("Vulkan driver file {}: {problem}", path.display()));
        }
    }
    eprintln!("If the GPU is visible (for example in nvidia-smi) but Vulkan sees only a software adapter, check the ICD driver file and its library, not just GPU passthrough. Run `VK_LOADER_DEBUG=error,warn vork setup` for loader diagnostics. See docs/REMOTE-GPU.md#vulkan-driver-discovery.");
    (!problems.is_empty()).then(|| problems.join("\n"))
}

#[cfg(target_os = "linux")]
fn inspect(path: &std::path::Path) -> Option<String> {
    let manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    let library = manifest.get("ICD")?.get("library_path")?.as_str()?;
    let mut resolved = std::path::PathBuf::from(library);
    // Bare sonames use the dynamic linker's search path; relative paths containing
    // a slash are relative to the manifest, per the Vulkan loader contract.
    if resolved.is_relative() && library.contains('/') {
        resolved = path.parent()?.join(resolved);
    }
    probe_library(&resolved)
        .err()
        .map(|cause| format!("points at {library}, which cannot load as a Vulkan driver: {cause}"))
}

#[cfg(target_os = "linux")]
fn probe_library(path: &std::path::Path) -> Result<(), String> {
    use std::{
        ffi::{CStr, CString},
        os::unix::ffi::OsStrExt,
    };
    let name = CString::new(path.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
    // SAFETY: paths and symbol names are NUL terminated. The handle stays open
    // while the Vulkan ABI function is called, and is closed on every path.
    unsafe {
        let handle = libc::dlopen(name.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
        if handle.is_null() {
            let error = libc::dlerror();
            return Err(if error.is_null() {
                "dlopen failed".into()
            } else {
                CStr::from_ptr(error).to_string_lossy().into_owned()
            });
        }
        let mut symbol = libc::dlsym(handle, c"vk_icdGetInstanceProcAddr".as_ptr());
        if symbol.is_null() {
            symbol = libc::dlsym(handle, c"vkGetInstanceProcAddr".as_ptr());
        }
        let result = if symbol.is_null() {
            Err("missing Vulkan instance entry point".into())
        } else {
            type GetProc = unsafe extern "system" fn(
                *mut libc::c_void,
                *const libc::c_char,
            ) -> *mut libc::c_void;
            let get_proc: GetProc = std::mem::transmute(symbol);
            if get_proc(std::ptr::null_mut(), c"vkCreateInstance".as_ptr()).is_null() {
                Err("driver does not expose vkCreateInstance (this can happen with libGLX_nvidia.so.0 in headless containers; see the EGL ICD setup in docs/REMOTE-GPU.md)".into())
            } else {
                Ok(())
            }
        };
        libc::dlclose(handle);
        result
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    #[test]
    fn broken_manifest_names_file_library_and_loader_cause() {
        let dir = std::env::temp_dir().join(format!("vork-icd-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("nvidia_icd.json");
        std::fs::write(&file, r#"{"ICD":{"library_path":"./missing-nvidia.so"}}"#).unwrap();
        let message = inspect(&file).unwrap();
        assert!(message.contains("./missing-nvidia.so"));
        assert!(message.contains("cannot load as a Vulkan driver"));
        assert!(message.contains(&dir.display().to_string()));
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn headless_driver_missing_create_instance_is_rejected() {
        let dir = std::env::temp_dir().join(format!("vork-headless-icd-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("driver.c");
        let library = dir.join("driver.so");
        std::fs::write(
            &source,
            "void *vk_icdGetInstanceProcAddr(void *instance, const char *name) { return 0; }",
        )
        .unwrap();
        assert!(std::process::Command::new("cc")
            .args(["-shared", "-fPIC"])
            .arg(&source)
            .arg("-o")
            .arg(&library)
            .status()
            .unwrap()
            .success());
        assert!(probe_library(&library)
            .unwrap_err()
            .contains("does not expose vkCreateInstance"));
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn non_vulkan_library_is_rejected() {
        assert!(probe_library(std::path::Path::new("libc.so.6"))
            .unwrap_err()
            .contains("missing Vulkan instance entry point"));
    }
}
