//! Opt-in benchmark observer over the pinned production TerminalCore library.
//! This observes a native buffer, never a GUI render or screen pixel.
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, VecDeque},
    ffi::c_void,
    os::windows::ffi::OsStrExt,
    path::Path,
    sync::{Arc, Mutex},
};
use windows_sys::Win32::Foundation::FreeLibrary;
use windows_sys::Win32::System::LibraryLoader::{
    GetProcAddress, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
};

type Response = unsafe extern "C" fn(*mut c_void, *const u16, u32);
type Create = unsafe extern "C" fn(u16, u16, u16, Response, *mut c_void) -> *mut c_void;
type Destroy = unsafe extern "C" fn(*mut c_void);
type Write = unsafe extern "C" fn(*mut c_void, *const u8, u32) -> i32;
type Snapshot = unsafe extern "C" fn(*mut c_void, *mut u16, u32, *mut u32) -> i32;
type Resize = unsafe extern "C" fn(*mut c_void, u16, u16) -> i32;
type Version = unsafe extern "C" fn() -> u32;

struct Library(windows_sys::Win32::Foundation::HMODULE);
impl Drop for Library {
    fn drop(&mut self) {
        // All model callbacks and native worker threads stop before this owner.
        unsafe { FreeLibrary(self.0) };
    }
}
impl Library {
    unsafe fn symbol<T: Copy>(&self, name: &[u8]) -> Result<T> {
        let address = unsafe { GetProcAddress(self.0, name.as_ptr()) }
            .context("native observer export missing")?;
        ensure!(std::mem::size_of::<T>() == std::mem::size_of_val(&address));
        // Call sites below supply the exact C ABI types from NativeModel.cpp.
        Ok(unsafe { std::mem::transmute_copy(&address) })
    }
}

#[derive(Default)]
struct Replies {
    frames: VecDeque<Vec<u8>>,
    error: Option<String>,
}
unsafe extern "C" fn response(context: *mut c_void, text: *const u16, length: u32) {
    if context.is_null() || length == 0 {
        return;
    }
    // The boxed queue outlives the C++ model and every callback it can issue.
    let queue = unsafe { &*context.cast::<Mutex<Replies>>() };
    let mut queue = queue.lock().unwrap_or_else(|error| error.into_inner());
    if text.is_null() || length > 1024 * 1024 || queue.frames.len() >= 256 {
        queue.error = Some("invalid or overflowing native response queue".into());
        return;
    }
    match String::from_utf16(unsafe { std::slice::from_raw_parts(text, length as usize) }) {
        Ok(value) => queue.frames.push_back(value.into_bytes()),
        Err(error) => queue.error = Some(error.to_string()),
    }
}

pub(crate) struct NativeObserver {
    handle: *mut c_void,
    destroy: Destroy,
    write: Write,
    snapshot: Snapshot,
    resize: Resize,
    replies: Box<Mutex<Replies>>,
    identity: Value,
    failure: Option<String>,
    _library: Library,
}
// All C ABI calls are serialized by the outer Rust mutex. Native callbacks
// only enqueue replies; they never borrow the model or enter the PTY writer.
unsafe impl Send for NativeObserver {}
impl Drop for NativeObserver {
    fn drop(&mut self) {
        unsafe { (self.destroy)(self.handle) };
    }
}
impl NativeObserver {
    pub(crate) fn configured(
        environment: &BTreeMap<String, String>,
    ) -> Result<Option<Arc<Mutex<Self>>>> {
        let effective = |name: &str| {
            environment
                .get(name)
                .cloned()
                .or_else(|| std::env::var(name).ok())
        };
        let Some(path) = effective("BLUEBERRY_NATIVE_MODEL_DLL").filter(|value| !value.is_empty())
        else {
            ensure!(
                effective("BLUEBERRY_NATIVE_OVERLAY").as_deref() != Some("4"),
                "native companion timing requires a frozen native-buffer observer; the legacy observer cannot measure it"
            );
            return Ok(None);
        };
        let expected = effective("BLUEBERRY_NATIVE_MODEL_SHA256")
            .context("native observer requires a frozen DLL digest")?;
        let path = Path::new(&path).canonicalize()?;
        let actual = format!("{:X}", Sha256::digest(std::fs::read(&path)?));
        ensure!(
            actual.eq_ignore_ascii_case(&expected),
            "native observer DLL identity mismatch"
        );
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
        let handle = unsafe {
            LoadLibraryExW(
                wide.as_ptr(),
                std::ptr::null_mut(),
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
            )
        };
        ensure!(
            !handle.is_null(),
            "load native observer: {}",
            std::io::Error::last_os_error()
        );
        let library = Library(handle);
        let version: Version = unsafe { library.symbol(b"bb_native_model_version\0")? };
        let overlay_version: Version =
            unsafe { library.symbol(b"bb_native_model_overlay_version\0")? };
        ensure!(
            unsafe { version() } == 1 && unsafe { overlay_version() } == 4,
            "native observer ABI or companion interface mismatch"
        );
        let create: Create = unsafe { library.symbol(b"bb_native_model_create\0")? };
        let destroy: Destroy = unsafe { library.symbol(b"bb_native_model_destroy\0")? };
        let write: Write = unsafe { library.symbol(b"bb_native_model_write\0")? };
        let snapshot: Snapshot = unsafe { library.symbol(b"bb_native_model_snapshot\0")? };
        let resize: Resize = unsafe { library.symbol(b"bb_native_model_resize\0")? };
        let mut replies = Box::new(Mutex::new(Replies::default()));
        let model = unsafe {
            create(
                30,
                120,
                9001,
                response,
                (&mut *replies as *mut Mutex<Replies>).cast(),
            )
        };
        ensure!(
            !model.is_null(),
            "native observer model construction failed"
        );
        Ok(Some(Arc::new(Mutex::new(Self {
            handle: model,
            destroy,
            write,
            snapshot,
            resize,
            replies,
            identity: json!({"backend":"production_terminalcore_buffer","abi_version":1,"companion_interface_version":4,
                "dll":path,"dll_sha256":actual,"rows":30,"columns":120,"history_rows":9001,
                "renderer":"DummyRenderer","gui_renderer":false,"screen_pixels":"not_measured",
                "timestamp":"after_native_parser_write_and_snapshot","snapshot_binding":"per_received_chunk",
                "eof_dispatch":"explicit_model_api_boundaries",
                "startup_origin":"before_child_spawn_after_observer_initialization"}),
            _library: library,
            failure: None,
        }))))
    }
    pub(crate) fn identity(&self) -> Value {
        self.identity.clone()
    }
    pub(crate) fn write(&mut self, data: &[u8]) -> Result<()> {
        ensure!(
            data.len() <= 1024 * 1024,
            "native observer write bound exceeded"
        );
        let status = unsafe { (self.write)(self.handle, data.as_ptr(), data.len() as u32) };
        if status < 0 {
            self.failure = Some(format!("native observer write failed: {status:#x}"));
            bail!(
                "{}",
                self.failure.as_deref().unwrap_or("native write failed")
            );
        }
        self.check_error()?;
        Ok(())
    }
    pub(crate) fn snapshot(&mut self) -> Result<String> {
        let result = self.snapshot_inner();
        if let Err(error) = &result {
            self.failure = Some(format!("native observer snapshot: {error:#}"));
        }
        result
    }
    fn snapshot_inner(&self) -> Result<String> {
        self.check_error()?;
        for _ in 0..3 {
            let mut needed = 0;
            let status =
                unsafe { (self.snapshot)(self.handle, std::ptr::null_mut(), 0, &mut needed) };
            ensure!(
                status == 0x8007007au32 as i32 && (1..=2 * 1024 * 1024).contains(&needed),
                "native observer snapshot size failed: {status:#x}"
            );
            let mut text = vec![0u16; needed as usize];
            let status =
                unsafe { (self.snapshot)(self.handle, text.as_mut_ptr(), needed, &mut needed) };
            if status == 0x8007007au32 as i32 {
                continue;
            }
            ensure!(
                status >= 0 && needed > 0 && needed as usize <= text.len(),
                "native observer snapshot failed: {status:#x}"
            );
            return Ok(String::from_utf16(&text[..needed as usize - 1])?
                .trim_end()
                .to_owned());
        }
        bail!("native observer snapshot kept changing across bounded retries")
    }
    pub(crate) fn resize(&mut self, rows: u16, columns: u16) -> Result<()> {
        let status = unsafe { (self.resize)(self.handle, rows, columns) };
        ensure!(status >= 0, "native observer resize failed: {status:#x}");
        Ok(())
    }
    pub(crate) fn replies(&self) -> Result<Vec<Vec<u8>>> {
        let mut replies = self
            .replies
            .lock()
            .map_err(|_| anyhow::anyhow!("native reply queue poisoned"))?;
        if let Some(error) = &replies.error {
            bail!("{error}");
        }
        Ok(replies.frames.drain(..).collect())
    }
    pub(crate) fn check_error(&self) -> Result<()> {
        if let Some(error) = &self.failure {
            bail!("{error}");
        }
        let replies = self
            .replies
            .lock()
            .map_err(|_| anyhow::anyhow!("native reply queue poisoned"))?;
        if let Some(error) = &replies.error {
            bail!("{error}");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_interface_without_model_cannot_silently_use_legacy_observation() {
        let environment = BTreeMap::from([
            ("BLUEBERRY_NATIVE_OVERLAY".into(), "4".into()),
            ("BLUEBERRY_NATIVE_MODEL_DLL".into(), String::new()),
        ]);
        let error = NativeObserver::configured(&environment)
            .err()
            .expect("missing model accepted");
        assert!(error.to_string().contains("legacy observer cannot measure"));
    }
    #[test]
    fn mismatched_digest_is_rejected_before_loading_any_code() {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), b"not a DLL").unwrap();
        let environment = BTreeMap::from([
            (
                "BLUEBERRY_NATIVE_MODEL_DLL".into(),
                file.path().to_string_lossy().into_owned(),
            ),
            ("BLUEBERRY_NATIVE_MODEL_SHA256".into(), "0".repeat(64)),
        ]);
        let error = NativeObserver::configured(&environment)
            .err()
            .expect("wrong digest accepted");
        assert!(error.to_string().contains("identity mismatch"));
    }
}
