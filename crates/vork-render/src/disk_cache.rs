//! The renderer's single persistent store. Callers own content keys and payload
//! formats; this module owns location, integrity, atomic publication and eviction.
use crate::constants::{DISK_CACHE_BUDGET, DISK_CACHE_HEADROOM_DIVISOR};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::SystemTime,
};

#[derive(Clone)]
pub struct Store {
    root: PathBuf,
    identity: blake3::Hash,
}
impl Store {
    #[cfg(test)]
    pub(crate) fn for_test(root: PathBuf) -> Self {
        Self { root, identity: blake3::hash(b"isolated-test-store") }
    }
    /// Host preprocessing shares the renderer's store, checksums and eviction.
    pub fn host() -> Self {
        let store=Self {root:std::env::var_os("VORK_CACHE_DIR").map(PathBuf::from).unwrap_or_else(default_root),identity:blake3::hash(b"host-preprocessing")};
        store.trim();store
    }
    pub fn new(adapter: &wgpu::AdapterInfo) -> Self {
        let device_identity = blake3::hash(format!("{adapter:?}").as_bytes());
        let mut identity = blake3::Hasher::new();
        identity.update(env!("VORK_BUILD_ID").as_bytes());
        identity.update(device_identity.as_bytes());
        let store = Self {
            identity: identity.finalize(),
            root: std::env::var_os("VORK_CACHE_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(default_root),
        };
        store.trim();
        store
    }
    pub fn path(&self, kind: &str, version: u32, hash: blake3::Hash) -> PathBuf {
        let mut key = blake3::Hasher::new();
        // CPU decoding is independent of renderer builds and adapters. Codec
        // revisions belong in the caller's explicit format version.
        match kind {
            "decoded-texture" | "native-decoded-texture" => {}
            // Slang emits native source/IR, not device machine code. Generated
            // scene text is the caller's key; embedded modules, generators and
            // pinned compiler inputs have their own content identity.
            "slang-variant" => {
                key.update(env!("VORK_SHADER_ID").as_bytes());
            }
            "slang-program" | "slang-resolve" | "aov-resolve" => {
                key.update(env!("VORK_PORTABLE_ID").as_bytes());
            }
            "compressed-texture" => {
                key.update(env!("VORK_TEXTURE_BUILD_ID").as_bytes());
            }
            _ => {
                key.update(self.identity.as_bytes());
            }
        }
        key.update(hash.as_bytes());
        key.update(kind.as_bytes());
        key.update(&version.to_le_bytes());
        self.root
            .join(format!("{}.{kind}-v{version}.entry", key.finalize()))
    }
    /// Also trim on startup/shutdown, including stores left by older builds.
    pub fn trim(&self) {
        let _ = (|| -> std::io::Result<()> {
            let _lock = lock_store(&self.root)?;
            self.make_room(0)?;
            Ok(())
        })();
    }
    fn make_room(&self, incoming: u64) -> std::io::Result<bool> {
        let mut files = Vec::new();
        for entry in fs::read_dir(&self.root)?.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|ext| ext != "entry") {
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if metadata.is_file() {
                files.push((metadata.modified()?, metadata.len(), path));
            }
        }
        let bytes = files.iter().map(|(_, len, _)| len).sum::<u64>();
        let configured = std::env::var("VORK_CACHE_BYTES")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(DISK_CACHE_BUDGET);
        let budget = available_space(&self.root).map_or(configured, |free| {
            configured.min(bytes.saturating_add(free) / DISK_CACHE_HEADROOM_DIVISOR)
        });
        if incoming > budget {
            return Ok(false);
        }
        let remaining = self.trim_to_budget(files, bytes, budget - incoming);
        Ok(remaining <= budget - incoming)
    }
    fn trim_to_budget(
        &self,
        mut files: Vec<(SystemTime, u64, PathBuf)>,
        mut bytes: u64,
        budget: u64,
    ) -> u64 {
        if bytes <= budget {
            return bytes;
        }
        files.sort_unstable_by_key(|(used, _, _)| *used);
        for (_, len, path) in files {
            if bytes <= budget {
                break;
            }
            if fs::remove_file(path).is_ok() {
                bytes -= len;
            }
        }
        bytes
    }
}

/// Optional captures reserve their peak owned bytes before allocating readback
/// buffers. Outstanding content keys coalesce across producers, including the
/// active writer while it waits for the filesystem. This never limits rendering.
#[derive(Default)]
struct PersistenceState {
    bytes: usize,
    keys: std::collections::HashSet<PathBuf>,
}
struct PersistenceAdmission {
    state: std::sync::Mutex<PersistenceState>,
    capacity: usize,
}
pub(crate) struct PersistenceReservation {
    admission: std::sync::Arc<PersistenceAdmission>,
    key: PathBuf,
    bytes: usize,
}
impl PersistenceAdmission {
    fn reserve(self: &std::sync::Arc<Self>, key: &Path, bytes: usize) -> Option<PersistenceReservation> {
        let mut state=self.state.try_lock().ok()?;
        if bytes==0 || bytes>self.capacity.saturating_sub(state.bytes) || state.keys.contains(key) {return None;}
        state.keys.insert(key.to_owned());state.bytes+=bytes;
        Some(PersistenceReservation {admission:self.clone(),key:key.to_owned(),bytes})
    }
}
fn persistence_admission() -> &'static std::sync::Arc<PersistenceAdmission> {
    static ADMISSION:std::sync::OnceLock<std::sync::Arc<PersistenceAdmission>>=std::sync::OnceLock::new();
    ADMISSION.get_or_init(||std::sync::Arc::new(PersistenceAdmission {
        state:Default::default(),capacity:crate::constants::CACHE_PERSISTENCE_BYTES,
    }))
}
pub(crate) fn reserve_persistence(key: &Path, bytes: usize) -> Option<PersistenceReservation> {
    persistence_admission().reserve(key,bytes)
}
impl Drop for PersistenceReservation {
    fn drop(&mut self) {
        let mut state=self.admission.state.lock().unwrap();
        state.bytes-=self.bytes;state.keys.remove(&self.key);
    }
}
impl PersistenceReservation {
    pub(crate) fn publish(self:std::sync::Arc<Self>, task: impl FnOnce() + Send + 'static) {
        persist(move || {let _reservation=self;task();});
    }
}
#[cfg(test)]
pub(crate) fn persistence_bytes() -> usize {persistence_admission().state.lock().unwrap().bytes}

/// Optional persistence owns its payload after GPU readback. One process worker
/// serializes publication through the ordinary store lock and eviction policy.
/// Enqueueing never waits for the filesystem; process exit may discard pending work.
fn persist(task: impl FnOnce() + Send + 'static) {
    type Task = Box<dyn FnOnce() + Send>;
    static WORKER: std::sync::OnceLock<Option<std::sync::mpsc::Sender<Task>>> = std::sync::OnceLock::new();
    if let Some(sender) = WORKER.get_or_init(|| {
        let (sender, receiver) = std::sync::mpsc::channel::<Task>();
        std::thread::Builder::new().name("vork-cache-persistence".into()).spawn(move || {
            for task in receiver { task(); }
        }).ok().map(|_| sender)
    }) {
        let _ = sender.send(Box::new(task));
    }
}

#[cfg(test)]
pub(crate) fn wait_persistence() -> anyhow::Result<()> {
    let (done,wait)=std::sync::mpsc::channel();
    persist(move || {let _=done.send(());});
    Ok(wait.recv_timeout(std::time::Duration::from_secs(30))?)
}

/// Writers and eviction share a persistent advisory lock across processes. Never
/// unlink the lock file: existing waiters must keep locking the same inode.
fn lock_store(root: &Path) -> std::io::Result<fs::File> {
    fs::create_dir_all(root)?;
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join("store.lock"))?;
    file.lock()?;
    // A killed writer can leave at most this one reservation. No live writer
    // can own it while we hold the store lock.
    match fs::remove_file(root.join("store.tmp")) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    Ok(file)
}

/// Platform cache location for the same content-addressed store and trim policy.
fn default_root() -> PathBuf {
    #[cfg(target_os = "macos")]
    if let Some(home) = std::env::var_os("HOME").filter(|value| !value.is_empty()) {
        return PathBuf::from(home).join("Library/Caches/vork");
    }
    #[cfg(windows)]
    if let Some(local) = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
    {
        return local.join("vork");
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        if let Some(cache) = std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
        {
            return cache.join("vork");
        }
        if let Some(home) = std::env::var_os("HOME").filter(|value| !value.is_empty()) {
            return PathBuf::from(home).join(".cache/vork");
        }
    }
    // Unusual service accounts without a home retain best-effort caching.
    std::env::temp_dir().join("vork")
}

/// A damaged, absent or oversized entry is an ordinary cache miss.
pub fn read(path: &Path) -> Option<Vec<u8>> {
    read_with_shaders(path, shipped_shaders().as_deref())
}

/// Release assets are an immutable base of the same content-addressed store.
/// Never accept driver blobs, known-pipeline ledgers or preprocessing entries
/// from a different machine. Missing or damaged assets compile on demand.
fn read_with_shaders(path: &Path, shaders: Option<&Path>) -> Option<Vec<u8>> {
    read_bounded(path, usize::MAX as u64).or_else(|| {
        let name = path.file_name()?.to_str()?;
        let (key, kind) = name.split_once('.')?;
        if key.len() != 64 || !key.bytes().all(|b| b.is_ascii_hexdigit())
            || !matches!(kind, "slang-variant-v1.entry" | "slang-program-v2.entry"
                | "slang-resolve-v2.entry" | "aov-resolve-v1.entry") {
            return None;
        }
        read_bounded(&shaders?.join(name), usize::MAX as u64)
    })
}

fn shipped_shaders() -> Option<PathBuf> {
    static DIRECTORY: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    DIRECTORY.get_or_init(|| Some(std::env::current_exe().ok()?.canonicalize().ok()?.parent()?.join("shaders"))).clone()
}

/// Immutable cache entries are atomically published and never modified in place.
/// Keep the mapping alive through decoding/upload, including after eviction.
pub(crate) struct MappedPayload(memmap2::Mmap);

impl std::ops::Deref for MappedPayload {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.0[blake3::OUT_LEN..]
    }
}

pub(crate) fn read_mapped(path: &Path) -> Option<MappedPayload> {
    #[cfg(not(windows))]
    let file = fs::File::open(path).ok()?;
    #[cfg(windows)]
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .or_else(|_| fs::File::open(path))
        .ok()?;
    // SAFETY: the store only publishes new files by rename, and eviction unlinks
    // them. Neither operation mutates the opened file backing this mapping.
    let mapped = unsafe { memmap2::Mmap::map(&file) }.ok()?;
    let (digest, payload) = mapped.split_at_checked(blake3::OUT_LEN)?;
    if blake3::hash(payload).as_bytes() != digest {
        return None;
    }
    let _ = file.set_times(fs::FileTimes::new().set_modified(SystemTime::now()));
    Some(MappedPayload(mapped))
}

pub(crate) fn read_bounded(path: &Path, limit: u64) -> Option<Vec<u8>> {
    #[cfg(not(windows))]
    let mut file = fs::File::open(path).ok()?;
    // Windows SetFileTime requires write access. Read-only stores can still hit,
    // even when their last-use timestamp cannot be refreshed.
    #[cfg(windows)]
    let mut file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .or_else(|_| fs::File::open(path))
        .ok()?;
    let length = file.metadata().ok()?.len();
    if !(blake3::OUT_LEN as u64..=limit).contains(&length) {
        return None;
    }
    let mut digest = [0; blake3::OUT_LEN];
    file.read_exact(&mut digest).ok()?;
    let payload_len = usize::try_from(length).ok()?.checked_sub(blake3::OUT_LEN)?;
    let mut payload = Vec::new();
    payload.try_reserve_exact(payload_len).ok()?;
    payload.resize(payload_len, 0);
    file.read_exact(&mut payload).ok()?;
    if blake3::hash(&payload).as_bytes() != &digest {
        return None;
    }
    // Touch only metadata on a hit, never rewrite a large pixel payload.
    let _ = file.set_times(fs::FileTimes::new().set_modified(SystemTime::now()));
    Some(payload)
}

/// Driver blobs obey the shared store budget, just like compiler artifacts.
/// There is no independent entry cap: a complete driver blob cannot be truncated.
pub(crate) fn read_pipeline(path: &Path) -> Option<Vec<u8>> {
    read(path)
}

pub(crate) fn write_pipeline(path: &Path, payload: &[u8]) -> std::io::Result<bool> {
    publish_parts(path, &[payload])
}

pub fn write(path: &Path, payload: &[u8]) -> std::io::Result<()> {
    write_parts(path, &[payload])
}

/// Serialize existing buffers directly, without copying decoded pixel allocations.
pub(crate) fn write_parts(path: &Path, parts: &[&[u8]]) -> std::io::Result<()> {
    publish_parts(path, parts).map(|_| ())
}

fn publish_parts(path: &Path, parts: &[&[u8]]) -> std::io::Result<bool> {
    let root = path.parent().unwrap();
    let _lock = lock_store(root)?;
    let incoming = parts
        .iter()
        .try_fold(blake3::OUT_LEN as u64, |n, p| n.checked_add(p.len() as u64))
        .ok_or_else(|| std::io::Error::other("cache payload overflow"))?;
    let store = Store {
        root: root.to_owned(),
        identity: blake3::hash(b""),
    };
    if !store.make_room(incoming)? {
        return Ok(false);
    }
    let temporary = root.join("store.tmp");
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        let mut digest = blake3::Hasher::new();
        for part in parts {
            digest.update(part);
        }
        file.write_all(digest.finalize().as_bytes())?;
        for part in parts {
            file.write_all(part)?;
        }
        drop(file);
        fs::rename(&temporary, path)
    })();
    let _ = fs::remove_file(temporary);
    result.map(|_| true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn optional_persistence_reservations_bound_bytes_coalesce_and_release() {
        let admission=std::sync::Arc::new(PersistenceAdmission {state:Default::default(),capacity:100});
        let key=Path::new("one");
        let first=admission.reserve(key,60).unwrap();
        assert!(admission.reserve(key,1).is_none());
        assert!(admission.reserve(Path::new("two"),41).is_none());
        let second=admission.reserve(Path::new("two"),40).unwrap();
        assert_eq!(admission.state.lock().unwrap().bytes,100);
        drop(first);
        assert!(admission.reserve(key,60).is_some());
        drop(second);
        assert_eq!(admission.state.lock().unwrap().bytes,0);
        let lock=admission.state.lock().unwrap();
        assert!(admission.reserve(key,1).is_none(),"admission must not block on contention");
        drop(lock);
        assert!(admission.reserve(key,100).is_some());
    }

    #[test]
    fn mapped_payload_rejects_corrupt_and_truncated_entries() {
        let root = std::env::temp_dir().join(format!("vork-mapped-test-{}", std::process::id()));
        let path = root.join("test.entry");
        write(&path, b"pixels").unwrap();
        assert_eq!(&*read_mapped(&path).unwrap(), b"pixels");
        fs::write(&path, [0; blake3::OUT_LEN + 6]).unwrap();
        assert!(read_mapped(&path).is_none());
        fs::write(&path, [0; blake3::OUT_LEN - 1]).unwrap();
        assert!(read_mapped(&path).is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn portable_artifacts_share_content_across_builds_and_devices() {
        let a = Store {
            root: PathBuf::from("cache"),
            identity: blake3::hash(b"build A device A"),
        };
        let b = Store {
            root: a.root.clone(),
            identity: blake3::hash(b"build B device B"),
        };
        let content = blake3::hash(b"image");
        for kind in [
            "decoded-texture",
            "native-decoded-texture",
            "compressed-texture",
        ] {
            assert_eq!(a.path(kind, 2, content), b.path(kind, 2, content));
        }
        assert_eq!(
            a.path("slang-variant", 1, content),
            b.path("slang-variant", 1, content)
        );
        for kind in ["slang-program", "slang-resolve", "aov-resolve"] {
            assert_eq!(a.path(kind, 1, content), b.path(kind, 1, content));
        }
        assert_ne!(a.path("pipeline", 1, content), b.path("pipeline", 1, content));
        assert_ne!(
            a.path("slang-variant", 1, content),
            a.path("slang-variant", 1, blake3::hash(b"changed shader"))
        );
    }

    #[test]
    fn shipped_shader_fallback_checks_integrity_and_excludes_device_state() {
        let root = std::env::temp_dir().join(format!("vork-shipped-shaders-{}", std::process::id()));
        let store = Store::for_test(root.join("cache"));
        let shaders = root.join("shaders");
        for kind in ["slang-variant", "slang-program", "slang-resolve", "aov-resolve", "pipeline", "pipeline-keys", "shader-warm"] {
            let version = if matches!(kind, "slang-resolve" | "slang-program") {2} else {1};
            let path = store.path(kind, version, blake3::hash(b"fixture"));
            let shipped = shaders.join(path.file_name().unwrap());
            write(&shipped, b"shipped").unwrap();
            let portable = matches!(kind, "slang-variant" | "slang-program" | "slang-resolve" | "aov-resolve");
            assert_eq!(read_with_shaders(&path, Some(&shaders)), portable.then(|| b"shipped".to_vec()));
            write(&path, b"local").unwrap();
            assert_eq!(read_with_shaders(&path, Some(&shaders)).unwrap(), b"local");
            fs::write(&path, b"corrupt").unwrap();
            assert_eq!(read_with_shaders(&path, Some(&shaders)), portable.then(|| b"shipped".to_vec()));
            fs::write(&shipped, b"corrupt").unwrap();
            assert!(read_with_shaders(&path, Some(&shaders)).is_none());
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn concurrent_process_budget_worker() {
        let Some(root) = std::env::var_os("VORK_CACHE_TEST_ROOT") else {
            return;
        };
        let root = PathBuf::from(root);
        for i in 0..24 {
            let path = root.join(format!("{}-{i}.entry", std::process::id()));
            let payload = vec![i as u8; 128];
            write(&path, &payload).unwrap();
            // Another writer may evict this entry, but a hit must be complete.
            if let Some(hit) = read(&path) {
                assert_eq!(hit, payload);
            }
        }
    }

    #[test]
    fn concurrent_processes_obey_budget_and_recover_interrupted_writer() {
        let root = std::env::temp_dir().join(format!("vork-budget-test-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("store.tmp"), b"interrupted write").unwrap();
        let mut children: Vec<_> = (0..4)
            .map(|_| {
                std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "disk_cache::tests::concurrent_process_budget_worker",
                    ])
                    .env("VORK_CACHE_TEST_ROOT", &root)
                    .env("VORK_CACHE_BYTES", "1024")
                    .spawn()
                    .unwrap()
            })
            .collect();
        for child in &mut children {
            assert!(child.wait().unwrap().success());
        }
        assert!(!root.join("store.tmp").exists());
        let bytes: u64 = fs::read_dir(&root)
            .unwrap()
            .flatten()
            .filter(|entry| entry.path().extension().is_some_and(|e| e == "entry"))
            .map(|entry| entry.metadata().unwrap().len())
            .sum();
        assert!(bytes > 0 && bytes <= 1024, "{bytes}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn opened_reader_survives_eviction_and_hits_refresh_lru() {
        let root = std::env::temp_dir().join(format!("vork-reader-test-{}", std::process::id()));
        let store = Store {
            root: root.clone(),
            identity: blake3::hash(b"test"),
        };
        let old = store.path("texture", 1, blake3::hash(b"old"));
        let recent = store.path("texture", 1, blake3::hash(b"recent"));
        write(&old, b"old").unwrap();
        write(&recent, b"recent").unwrap();
        let mapped = read_mapped(&recent).unwrap();
        for path in [&old, &recent] {
            fs::OpenOptions::new()
                .write(true)
                .open(path)
                .unwrap()
                .set_times(fs::FileTimes::new().set_modified(SystemTime::UNIX_EPOCH))
                .unwrap();
        }
        assert_eq!(read(&old).unwrap(), b"old");
        let mut opened = fs::File::open(&recent).unwrap();
        let files = [&old, &recent]
            .into_iter()
            .map(|path| {
                let meta = fs::metadata(path).unwrap();
                (meta.modified().unwrap(), meta.len(), path.clone())
            })
            .collect();
        store.trim_to_budget(files, 73, 35);
        assert!(old.exists());
        assert!(!recent.exists());
        let mut bytes = Vec::new();
        opened.read_to_end(&mut bytes).unwrap();
        assert_eq!(&bytes[blake3::OUT_LEN..], b"recent");
        assert_eq!(&*mapped, b"recent");
        drop(mapped);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn pipeline_blob_survives_beyond_the_former_entry_cap() {
        let root = std::env::temp_dir().join(format!("vork-pipeline-large-{}", std::process::id()));
        let path = root.join("driver.entry");
        let payload = vec![7; 64 * 1024 * 1024 + 1];
        assert!(write_pipeline(&path, &payload).unwrap());
        assert_eq!(read_pipeline(&path).unwrap(), payload);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn pipeline_budget_worker() {
        let Some(root) = std::env::var_os("VORK_PIPELINE_TEST_ROOT") else { return; };
        let path = PathBuf::from(root).join("driver.entry");
        write_pipeline(&path, b"previous").unwrap();
        assert!(!write_pipeline(&path, &[0; 1024]).unwrap());
        assert_eq!(read_pipeline(&path).unwrap(), b"previous");
    }

    #[test]
    fn pipeline_budget_failure_preserves_previous_blob() {
        let root = std::env::temp_dir().join(format!("vork-pipeline-budget-{}", std::process::id()));
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "disk_cache::tests::pipeline_budget_worker"])
            .env("VORK_PIPELINE_TEST_ROOT", &root).env("VORK_CACHE_BYTES", "128")
            .status().unwrap();
        assert!(status.success());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn atomic_writers_and_corrupt_entry_recovery() {
        let root = std::env::temp_dir().join(format!("vork-store-test-{}", std::process::id()));
        let store = Store {
            root: root.clone(),
            identity: blake3::hash(b"build + adapter"),
        };
        let hash = blake3::hash(b"source");
        let path = store.path("texture", 1, hash);
        assert_ne!(path, store.path("texture", 2, hash));
        assert_ne!(path, store.path("pipeline", 1, hash));
        assert_ne!(
            path,
            store.path("texture", 1, blake3::hash(b"different source"))
        );
        assert!(read(&path).is_none());
        std::thread::scope(|scope| {
            for payload in [b"writer A", b"writer B"] {
                let path = &path;
                scope.spawn(move || write(path, payload).unwrap());
            }
        });
        let data = read(&path).unwrap();
        assert!(data == b"writer A" || data == b"writer B");
        let mut broken = fs::read(&path).unwrap();
        *broken.last_mut().unwrap() ^= 1;
        fs::write(&path, broken).unwrap();
        assert!(read(&path).is_none());
        // Process exit during a background publication leaves only an ignored
        // reservation. The next writer recovers it under the store lock.
        fs::write(root.join("store.tmp"), b"partial background artifact").unwrap();
        assert!(read(&path).is_none());
        write(&path, b"recomputed").unwrap();
        assert_eq!(read(&path).unwrap(), b"recomputed");
        assert_eq!(fs::read_dir(&root).unwrap().count(), 2);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn eviction_is_shared_across_artifact_kinds() {
        let root = std::env::temp_dir().join(format!("vork-trim-test-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let store = Store {
            root: root.clone(),
            identity: blake3::hash(b"build + adapter"),
        };
        let old = store.path("pipeline", 1, blake3::hash(b"old"));
        let recent = store.path("hdr", 1, blake3::hash(b"recent"));
        for (path, age) in [(&old, 1), (&recent, 2)] {
            let file = fs::File::create(path).unwrap();
            // Sparse files exercise the real budget without writing a GiB.
            file.set_len(513).unwrap();
            file.set_times(
                fs::FileTimes::new()
                    .set_modified(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(age)),
            )
            .unwrap();
        }
        let files = [&old, &recent]
            .into_iter()
            .map(|path| {
                let meta = fs::metadata(path).unwrap();
                (meta.modified().unwrap(), meta.len(), path.clone())
            })
            .collect();
        store.trim_to_budget(files, 1026, 1024);
        assert!(!old.exists());
        assert!(recent.exists());
        fs::remove_dir_all(root).unwrap();
    }
}

/// Query the filesystem containing the one cache; unavailable accounting skips eviction.
#[cfg(unix)]
fn available_space(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: path is NUL-terminated and stat points to writable storage.
    if unsafe { libc::statvfs(path.as_ptr(), stat.as_mut_ptr()) } != 0 {
        return None;
    }
    let stat = unsafe { stat.assume_init() };
    (stat.f_bavail as u64).checked_mul(stat.f_frsize as u64)
}
#[cfg(not(any(unix, windows)))]
fn available_space(_path: &Path) -> Option<u64> {
    None
}

#[cfg(windows)]
fn available_space(path: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetDiskFreeSpaceExW(
            directory: *const u16,
            available: *mut u64,
            total: *mut u64,
            free: *mut u64,
        ) -> i32;
    }
    let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut available = 0;
    // SAFETY: path is NUL-terminated; the output pointer is valid and optional outputs are null.
    let success = unsafe {
        GetDiskFreeSpaceExW(
            path.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    (success != 0).then_some(available)
}
