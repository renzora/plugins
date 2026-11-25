//! Asking a player to install .NET, for a game exported without it.
//!
//! An export can bundle the runtime (the default) or leave it to the player's
//! machine. When it is left out and the machine has none, the game still runs
//! with its C# doing nothing, and a line in a log is all anyone would see. So
//! the game asks, once per launch, in the operating system's own dialog: the
//! game window may not be up yet, and a native dialog needs nothing from the
//! engine. Saying yes opens Microsoft's download page in the browser.
//!
//! Shown from the plugin's build thread, so the game keeps starting while the
//! dialog is up.

use std::sync::Once;

use crate::dotnet::RUNTIME_DOWNLOAD;

static ASKED: Once = Once::new();

/// Ask once, without blocking the caller.
pub fn missing_runtime() {
    ASKED.call_once(|| {
        let _ = std::thread::Builder::new().name("csharp-runtime-prompt".into()).spawn(|| {
            let game = std::env::current_exe()
                .ok()
                .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
                .unwrap_or_else(|| "This game".into());
            let message = format!(
                "{game} needs the Microsoft .NET 10 Runtime, which is not installed on this computer.\n\n\
                 The game will start, but parts of it will not work until .NET is installed.\n\n\
                 Open the .NET download page now?"
            );
            if ask(&game, &message) {
                open(RUNTIME_DOWNLOAD);
            }
        });
    });
}

#[cfg(windows)]
fn ask(title: &str, message: &str) -> bool {
    type MessageBoxW = unsafe extern "system" fn(*mut std::ffi::c_void, *const u16, *const u16, u32) -> i32;
    const MB_YESNO: u32 = 0x4;
    const MB_ICONWARNING: u32 = 0x30;
    const MB_SETFOREGROUND: u32 = 0x1_0000;
    const IDYES: i32 = 6;
    let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    // SAFETY: user32 is part of every Windows install, and MessageBoxW has
    // this signature; both strings are NUL-terminated and outlive the call.
    unsafe {
        let Ok(lib) = libloading::Library::new("user32.dll") else { return false };
        // Never freed, as with every library this process opens: user32 is
        // loaded for good anyway, and a FreeLibrary here gains nothing.
        let lib: &'static libloading::Library = Box::leak(Box::new(lib));
        let Ok(f) = lib.get::<MessageBoxW>(b"MessageBoxW\0") else { return false };
        let (t, m) = (wide(title), wide(message));
        f(std::ptr::null_mut(), m.as_ptr(), t.as_ptr(), MB_YESNO | MB_ICONWARNING | MB_SETFOREGROUND) == IDYES
    }
}

#[cfg(target_os = "macos")]
fn ask(title: &str, message: &str) -> bool {
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!(
        "display dialog \"{}\" with title \"{}\" buttons {{\"Not now\", \"Download\"}} default button \"Download\" with icon caution",
        esc(message),
        esc(title)
    );
    std::process::Command::new("osascript")
        .args(["-e", &script])
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("Download"))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn ask(title: &str, message: &str) -> bool {
    // Whichever desktop dialog tool the system has; with neither, the log
    // line the plugin writes is all there is.
    if let Ok(s) = std::process::Command::new("zenity")
        .args(["--question", "--title", title, "--text", message, "--ok-label=Download", "--cancel-label=Not now"])
        .status()
    {
        return s.success();
    }
    std::process::Command::new("kdialog")
        .args(["--title", title, "--yesno", message])
        .status()
        .is_ok_and(|s| s.success())
}

fn open(url: &str) {
    #[cfg(windows)]
    let cmd = std::process::Command::new("rundll32").args(["url.dll,FileProtocolHandler", url]).spawn();
    #[cfg(target_os = "macos")]
    let cmd = std::process::Command::new("open").arg(url).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let cmd = std::process::Command::new("xdg-open").arg(url).spawn();
    if let Err(e) = cmd {
        bevy::log::warn!("[C#] could not open {url}: {e}");
    }
}
