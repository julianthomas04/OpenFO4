//! Windows Media Foundation, loaded when the menu video first needs it instead of linked: Windows N
//! editions lack it until the Media Feature Pack is installed, and a linked import would stop the
//! game from starting there.

use std::ffi::{CStr, c_void};
use std::path::Path;
use std::sync::OnceLock;

use windows::Win32::Foundation::HMODULE;
use windows::Win32::Media::MediaFoundation::{
    IMFMediaType, IMFSourceReader, MF_VERSION, MFSTARTUP_LITE,
};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::System::LibraryLoader::{
    GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
};
use windows::core::{HRESULT, HSTRING, Interface, PCSTR, PCWSTR};

use super::error::VideoError;

type Startup = unsafe extern "system" fn(u32, u32) -> HRESULT;
type Shutdown = unsafe extern "system" fn() -> HRESULT;
type CreateMediaType = unsafe extern "system" fn(*mut *mut c_void) -> HRESULT;
type CreateSourceReaderFromUrl =
    unsafe extern "system" fn(PCWSTR, *mut c_void, *mut *mut c_void) -> HRESULT;

/// The Media Foundation entry points the player uses.
pub struct MediaFoundation {
    startup: Startup,
    shutdown: Shutdown,
    create_media_type: CreateMediaType,
    create_source_reader: CreateSourceReaderFromUrl,
}

impl MediaFoundation {
    /// Loads Media Foundation once per process; the libraries stay loaded.
    pub fn get() -> Result<&'static Self, VideoError> {
        static LOADED: OnceLock<Result<MediaFoundation, String>> = OnceLock::new();
        LOADED
            .get_or_init(Self::load)
            .as_ref()
            .map_err(|reason| VideoError::MediaFoundationMissing(reason.clone()))
    }

    fn load() -> Result<Self, String> {
        let platform = library("mfplat.dll")?;
        let read_write = library("mfreadwrite.dll")?;
        // SAFETY: each symbol gets the signature the Windows SDK headers declare for it.
        unsafe {
            Ok(Self {
                startup: std::mem::transmute::<Symbol, Startup>(symbol(platform, c"MFStartup")?),
                shutdown: std::mem::transmute::<Symbol, Shutdown>(symbol(platform, c"MFShutdown")?),
                create_media_type: std::mem::transmute::<Symbol, CreateMediaType>(symbol(
                    platform,
                    c"MFCreateMediaType",
                )?),
                create_source_reader: std::mem::transmute::<Symbol, CreateSourceReaderFromUrl>(
                    symbol(read_write, c"MFCreateSourceReaderFromURL")?,
                ),
            })
        }
    }
}

type Symbol = unsafe extern "system" fn() -> isize;

fn library(name: &str) -> Result<HMODULE, String> {
    // SAFETY: loads a system library by name; nothing else is passed.
    unsafe { LoadLibraryExW(&HSTRING::from(name), None, LOAD_LIBRARY_SEARCH_SYSTEM32) }
        .map_err(|error| format!("cannot load {name}: {error}"))
}

fn symbol(module: HMODULE, name: &CStr) -> Result<Symbol, String> {
    // SAFETY: `module` is a loaded library and `name` is NUL-terminated.
    unsafe { GetProcAddress(module, PCSTR(name.as_ptr().cast())) }
        .ok_or_else(|| format!("{} is missing", name.to_string_lossy()))
}

/// Media Foundation started on the current thread, which joins the multithreaded COM apartment.
/// Both end when it is dropped, so it must outlive every Media Foundation object of the thread.
pub struct Session {
    api: &'static MediaFoundation,
}

impl Session {
    pub fn start(api: &'static MediaFoundation) -> Result<Self, VideoError> {
        // SAFETY: balanced by `CoUninitialize` below or in `drop`, on this thread.
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
            .ok()
            .map_err(failed("join the COM apartment"))?;
        // SAFETY: COM is initialised on this thread; balanced by `shutdown` in `drop`.
        if let Err(error) = unsafe { (api.startup)(MF_VERSION, MFSTARTUP_LITE) }.ok() {
            // SAFETY: balances the successful `CoInitializeEx` above.
            unsafe { CoUninitialize() };
            return Err(failed("start")(error));
        }
        Ok(Self { api })
    }

    pub fn create_media_type(&self) -> Result<IMFMediaType, VideoError> {
        let mut raw = std::ptr::null_mut();
        // SAFETY: the out pointer receives an owned reference on success.
        unsafe { (self.api.create_media_type)(&raw mut raw) }
            .ok()
            .map_err(failed("create a media type"))?;
        // SAFETY: success hands over one reference to a media type.
        Ok(unsafe { IMFMediaType::from_raw(raw) })
    }

    pub fn open(&self, path: &Path) -> Result<IMFSourceReader, VideoError> {
        let url = HSTRING::from(path.as_os_str());
        let mut raw = std::ptr::null_mut();
        // SAFETY: `url` is NUL-terminated and outlives the call; no attributes are passed.
        unsafe {
            (self.api.create_source_reader)(
                PCWSTR(url.as_ptr()),
                std::ptr::null_mut(),
                &raw mut raw,
            )
        }
        .ok()
        .map_err(failed("open the video"))?;
        // SAFETY: success hands over one reference to a source reader.
        Ok(unsafe { IMFSourceReader::from_raw(raw) })
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // SAFETY: balances the `startup` and `CoInitializeEx` of `start` on this thread.
        unsafe {
            if let Err(error) = (self.api.shutdown)().ok() {
                tracing::warn!("cannot shut Media Foundation down: {error}");
            }
            CoUninitialize();
        }
    }
}

/// Wraps a Media Foundation error with what was being attempted.
pub fn failed(action: &'static str) -> impl Fn(windows::core::Error) -> VideoError {
    move |error| VideoError::MediaFoundation {
        action,
        reason: error.to_string(),
    }
}
