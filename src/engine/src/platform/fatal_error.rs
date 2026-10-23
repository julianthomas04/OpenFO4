/// Reports an error that stops the program before any window or UI exists.
/// Debug builds have stderr; release builds use the GUI subsystem and have no
/// console, so the message goes into a native message box instead.
pub fn report(title: &str, message: &str) {
    eprintln!("fatal: {message}");
    if !cfg!(debug_assertions) {
        show_message_box(title, message);
    }
}

#[cfg(windows)]
fn show_message_box(title: &str, message: &str) {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};

    let wide = |text: &str| -> Vec<u16> {
        std::ffi::OsStr::new(text)
            .encode_wide()
            .chain(Some(0))
            .collect()
    };
    let (text, caption) = (wide(message), wide(title));
    // SAFETY: both buffers are NUL-terminated UTF-16 and outlive the call; a null owner window is allowed.
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            caption.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

#[cfg(not(windows))]
fn show_message_box(title: &str, message: &str) {
    eprintln!("{title}: {message}");
}
