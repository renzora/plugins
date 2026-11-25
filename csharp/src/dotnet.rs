//! Finding the machine's .NET, and starting its runtime inside this process.
//!
//! # The hosting flow
//!
//! This is Microsoft's documented "native host" flow, done by hand:
//!
//! 1. Find a .NET root: `DOTNET_ROOT`, else the default install location.
//! 2. Open `hostfxr` from `<root>/host/fxr/<newest version>/`, with
//!    `libloading`.
//! 3. `hostfxr_initialize_for_runtime_config` with the host assembly's
//!    `runtimeconfig.json` (net10.0, `rollForward: LatestMajor`, so a machine
//!    with only a newer .NET runs it too).
//! 4. `hostfxr_get_runtime_delegate(hdt_load_assembly_and_get_function_pointer)`.
//! 5. Ask that delegate for each `[UnmanagedCallersOnly]` entry point.
//!
//! `netcorehost` and `nethost` do the same and were ruled out: both fetch or
//! link Microsoft's prebuilt C++ `nethost` library when the plugin is compiled,
//! and a plugin here is compiled on the user's machine, with no C++ toolchain
//! guaranteed and no reason to need one. `hostfxr` is already on every machine
//! that has .NET, which is the only machine that can run C# anyway.
//!
//! # Once per process, and never shut down
//!
//! A process can host one .NET runtime, once: `hostfxr` refuses a second
//! initialisation with different settings, and there is no way to unload a
//! runtime at all. So [`Runtime`] is started at most once (lazily, by the first
//! C# script that needs it) and lives until the process exits; the library
//! handle is deliberately leaked, for the reason the engine never drops a
//! plugin's: every function pointer handed out points into it.
//!
//! # Errors are words
//!
//! Every failure here is a sentence a user can act on ("install the .NET 10
//! runtime from ..."), because it reaches them as the error on every C# script
//! in the project. `hostfxr` writes its own explanation through an error-writer
//! callback rather than returning it, so that callback is installed and its
//! text attached.
//!
//! This module depends on `std` and `libloading` alone, so the stand-alone test
//! harness includes it with `#[path]`.

#![allow(dead_code)]

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// What a user is told to install when no usable .NET is found.
pub const INSTALL_HINT: &str =
    "install the .NET 10 SDK (to edit and build C# scripts) or the .NET 10 runtime (to play a project already built), from https://dotnet.microsoft.com/download/dotnet/10.0";

/// Where a player installs .NET from, for a game that needs it.
pub const RUNTIME_DOWNLOAD: &str = "https://dotnet.microsoft.com/download/dotnet/10.0";

/// The oldest major version the plugin's C# targets; a bundled runtime is the
/// newest installed at or above it.
pub const MIN_MAJOR: u64 = 10;

/// A .NET installation: the root and what it holds.
#[derive(Clone, Debug)]
pub struct Dotnet {
    pub root: PathBuf,
    /// The newest `hostfxr` under `host/fxr/`.
    pub hostfxr: PathBuf,
    /// The `dotnet` driver, which `dotnet build` needs.
    pub exe: PathBuf,
    /// Whether an SDK is installed beside the runtime. Without one, scripts
    /// already built run but nothing can be compiled.
    pub has_sdk: bool,
}

/// The roots tried when `DOTNET_ROOT` is not set, in order. These are where
/// Microsoft's installers and the distributions' packages put .NET.
fn default_roots() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if cfg!(windows) {
        if let Some(pf) = std::env::var_os("ProgramFiles") {
            out.push(PathBuf::from(pf).join("dotnet"));
        }
        out.push(PathBuf::from(r"C:\Program Files\dotnet"));
    } else if cfg!(target_os = "macos") {
        out.push(PathBuf::from("/usr/local/share/dotnet"));
        out.push(PathBuf::from("/opt/homebrew/share/dotnet"));
    } else {
        out.push(PathBuf::from("/usr/share/dotnet"));
        out.push(PathBuf::from("/usr/lib/dotnet"));
        out.push(PathBuf::from("/usr/local/share/dotnet"));
    }
    // The per-user location `dotnet-install` scripts use.
    if let Some(home) = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }) {
        out.push(PathBuf::from(home).join(".dotnet"));
    }
    out
}

fn hostfxr_file() -> &'static str {
    if cfg!(windows) {
        "hostfxr.dll"
    } else if cfg!(target_os = "macos") {
        "libhostfxr.dylib"
    } else {
        "libhostfxr.so"
    }
}

/// `8.0.11` as numbers, for ordering. A pre-release suffix (`9.0.0-rc.2`)
/// keeps its leading numbers and sorts just below the release of the same
/// numbers, which is what "newest" should mean.
fn version_key(name: &str) -> (Vec<u64>, bool) {
    let (nums, pre) = match name.split_once('-') {
        Some((n, _)) => (n, true),
        None => (name, false),
    };
    let parts = nums.split('.').map(|p| p.parse::<u64>().unwrap_or(0)).collect();
    (parts, !pre)
}

/// The newest `host/fxr/<version>/hostfxr` under `root`.
fn newest_hostfxr(root: &Path) -> Option<PathBuf> {
    let fxr = root.join("host").join("fxr");
    let mut best: Option<((Vec<u64>, bool), PathBuf)> = None;
    for entry in std::fs::read_dir(&fxr).ok()?.flatten() {
        let lib = entry.path().join(hostfxr_file());
        if !lib.is_file() {
            continue;
        }
        let key = version_key(&entry.file_name().to_string_lossy());
        if best.as_ref().is_none_or(|(k, _)| key > *k) {
            best = Some((key, lib));
        }
    }
    best.map(|(_, p)| p)
}

impl Dotnet {
    /// The .NET at exactly `root`, if it holds a `hostfxr`: a game's bundled
    /// runtime, which is used whatever else the machine has.
    pub fn at(root: &Path) -> Option<Self> {
        let hostfxr = newest_hostfxr(root)?;
        Some(Self {
            root: root.to_path_buf(),
            hostfxr,
            exe: root.join(if cfg!(windows) { "dotnet.exe" } else { "dotnet" }),
            has_sdk: false,
        })
    }

    /// The files a game needs to run C# on this runtime with nothing
    /// installed: `hostfxr` and the newest `Microsoft.NETCore.App` at or
    /// above [`MIN_MAJOR`], laid out as a .NET root, with Microsoft's licence
    /// and notices (.NET is MIT licensed; the notices ship with it).
    ///
    /// Not the SDK, ASP.NET or the desktop frameworks: scripts reference only
    /// the base framework, and a runtime is about 70 MB where an SDK is many
    /// times that.
    pub fn runtime_files(&self) -> Result<Vec<(String, PathBuf)>, String> {
        let shared = self.root.join("shared").join("Microsoft.NETCore.App");
        let mut best: Option<((Vec<u64>, bool), PathBuf)> = None;
        for entry in std::fs::read_dir(&shared).map_err(|e| format!("{}: {e}", shared.display()))?.flatten() {
            let key = version_key(&entry.file_name().to_string_lossy());
            if key.0.first().copied().unwrap_or(0) < MIN_MAJOR || !entry.path().is_dir() {
                continue;
            }
            if best.as_ref().is_none_or(|(k, _)| key > *k) {
                best = Some((key, entry.path()));
            }
        }
        let (_, framework) = best.ok_or_else(|| {
            format!("this machine has no .NET {MIN_MAJOR} runtime to bundle (in {})", shared.display())
        })?;
        let mut out = Vec::new();
        let mut add_tree = |dir: &Path| {
            let mut stack = vec![dir.to_path_buf()];
            while let Some(d) = stack.pop() {
                let Ok(entries) = std::fs::read_dir(&d) else { continue };
                for e in entries.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        stack.push(p);
                    } else if let Ok(rel) = p.strip_prefix(&self.root) {
                        out.push((rel.to_string_lossy().replace('\\', "/"), p.clone()));
                    }
                }
            }
        };
        add_tree(&framework);
        if let Some(fxr_dir) = self.hostfxr.parent() {
            add_tree(fxr_dir);
        }
        for notice in ["LICENSE.txt", "ThirdPartyNotices.txt"] {
            let p = self.root.join(notice);
            if p.is_file() {
                out.push((notice.to_string(), p));
            }
        }
        out.sort();
        Ok(out)
    }

    /// Find a usable .NET: `DOTNET_ROOT` when set (and then only there, since
    /// someone set it deliberately), else the first default root holding a
    /// `hostfxr`.
    pub fn locate() -> Result<Self, String> {
        let explicit = std::env::var_os("DOTNET_ROOT").filter(|v| !v.is_empty());
        let roots = match &explicit {
            Some(root) => vec![PathBuf::from(root)],
            None => default_roots(),
        };
        for root in &roots {
            if let Some(hostfxr) = newest_hostfxr(root) {
                let exe = root.join(if cfg!(windows) { "dotnet.exe" } else { "dotnet" });
                let has_sdk = std::fs::read_dir(root.join("sdk"))
                    .map(|mut d| d.any(|e| e.is_ok_and(|e| e.path().is_dir())))
                    .unwrap_or(false);
                return Ok(Self {
                    root: root.clone(),
                    hostfxr,
                    exe,
                    has_sdk,
                });
            }
        }
        Err(match explicit {
            Some(root) => format!(
                "DOTNET_ROOT is {} but there is no .NET host under it ({}); {INSTALL_HINT}",
                PathBuf::from(root).display(),
                Path::new("host/fxr").display()
            ),
            None => format!("no .NET installation was found; {INSTALL_HINT}"),
        })
    }
}

// =============================================================================
// hostfxr
// =============================================================================

/// `char_t`: UTF-16 on Windows, UTF-8 elsewhere.
#[cfg(windows)]
type CharT = u16;
#[cfg(not(windows))]
type CharT = std::ffi::c_char;

/// A path as a NUL-terminated `char_t` string.
#[cfg(windows)]
fn host_str(s: &std::ffi::OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    s.encode_wide().chain(std::iter::once(0)).collect()
}
#[cfg(not(windows))]
fn host_str(s: &std::ffi::OsStr) -> Vec<std::ffi::c_char> {
    use std::os::unix::ffi::OsStrExt;
    s.as_bytes()
        .iter()
        .map(|b| *b as std::ffi::c_char)
        .chain(std::iter::once(0))
        .collect()
}

/// A NUL-terminated `char_t` string back to Rust.
///
/// # Safety
/// `p` must point at a NUL-terminated string.
unsafe fn from_host_str(p: *const CharT) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut len = 0;
    while *p.add(len) != 0 {
        len += 1;
    }
    let s = std::slice::from_raw_parts(p, len);
    #[cfg(windows)]
    {
        String::from_utf16_lossy(s)
    }
    #[cfg(not(windows))]
    {
        String::from_utf8_lossy(std::slice::from_raw_parts(s.as_ptr() as *const u8, len)).into_owned()
    }
}

/// `hostfxr_initialize_parameters`.
#[repr(C)]
struct InitParams {
    size: usize,
    host_path: *const CharT,
    dotnet_root: *const CharT,
}

type InitForConfigFn = unsafe extern "C" fn(*const CharT, *const InitParams, *mut *mut c_void) -> i32;
type GetDelegateFn = unsafe extern "C" fn(*mut c_void, i32, *mut *mut c_void) -> i32;
type CloseFn = unsafe extern "C" fn(*mut c_void) -> i32;
type ErrorWriterFn = unsafe extern "C" fn(*const CharT);
type SetErrorWriterFn = unsafe extern "C" fn(Option<ErrorWriterFn>) -> Option<ErrorWriterFn>;
/// `load_assembly_and_get_function_pointer_fn`. `CORECLR_DELEGATE_CALLTYPE`
/// is `__stdcall` on Windows, which on x64 and arm64 is the one convention.
type LoadAssemblyAndGetFnFn = unsafe extern "system" fn(
    assembly_path: *const CharT,
    type_name: *const CharT,
    method_name: *const CharT,
    delegate_type_name: *const CharT,
    reserved: *mut c_void,
    delegate: *mut *mut c_void,
) -> i32;

/// `hdt_load_assembly_and_get_function_pointer` in `hostfxr.h`.
const HDT_LOAD_ASSEMBLY_AND_GET_FUNCTION_POINTER: i32 = 5;
/// `UNMANAGEDCALLERSONLY_METHOD`: "the method is `[UnmanagedCallersOnly]`,
/// there is no delegate type".
const UNMANAGED_CALLERS_ONLY: *const CharT = usize::MAX as *const CharT;

/// What `hostfxr` wrote through the error writer while we were calling it.
static HOST_ERRORS: Mutex<String> = Mutex::new(String::new());

unsafe extern "C" fn error_writer(message: *const CharT) {
    let text = from_host_str(message);
    if let Ok(mut buf) = HOST_ERRORS.lock() {
        if !buf.is_empty() {
            buf.push(' ');
        }
        buf.push_str(text.trim());
    }
}

fn take_host_errors() -> String {
    HOST_ERRORS.lock().map(|mut b| std::mem::take(&mut *b)).unwrap_or_default()
}

/// A failure code with whatever `hostfxr` said about it.
fn failure(what: &str, code: i32) -> String {
    let said = take_host_errors();
    // 0x80008096 is FrameworkMissingFailure: the one a user can fix by
    // installing something, so it gets the instruction as well as the code.
    let hint = if code as u32 == 0x8000_8096 || said.contains("framework") {
        format!("; {INSTALL_HINT}")
    } else {
        String::new()
    };
    if said.is_empty() {
        format!("{what} failed (0x{:08x}){hint}", code as u32)
    } else {
        format!("{what} failed (0x{:08x}): {said}{hint}", code as u32)
    }
}

/// The .NET runtime, started in this process.
pub struct Runtime {
    load: LoadAssemblyAndGetFnFn,
}

// SAFETY: a function pointer into the runtime, which is process-global, never
// unloaded and callable from any thread.
unsafe impl Send for Runtime {}
unsafe impl Sync for Runtime {}

impl Runtime {
    /// Start the runtime described by `runtime_config` (the host assembly's
    /// `runtimeconfig.json`).
    pub fn start(dotnet: &Dotnet, runtime_config: &Path) -> Result<Self, String> {
        // SAFETY: loading a library runs its initialisers; hostfxr's are
        // Microsoft's and have no preconditions.
        let lib = unsafe { libloading::Library::new(&dotnet.hostfxr) }
            .map_err(|e| format!("could not open {}: {e}", dotnet.hostfxr.display()))?;
        // Never unloaded: see the module comment.
        let lib: &'static libloading::Library = Box::leak(Box::new(lib));

        // SAFETY: the symbol types are those `hostfxr.h` declares.
        let (init, get_delegate, close, set_writer) = unsafe {
            let init = *lib
                .get::<InitForConfigFn>(b"hostfxr_initialize_for_runtime_config\0")
                .map_err(|e| format!("{} is not a .NET host: {e}", dotnet.hostfxr.display()))?;
            let get_delegate = *lib
                .get::<GetDelegateFn>(b"hostfxr_get_runtime_delegate\0")
                .map_err(|e| format!("{} is not a .NET host: {e}", dotnet.hostfxr.display()))?;
            let close = *lib
                .get::<CloseFn>(b"hostfxr_close\0")
                .map_err(|e| format!("{} is not a .NET host: {e}", dotnet.hostfxr.display()))?;
            let set_writer = lib
                .get::<SetErrorWriterFn>(b"hostfxr_set_error_writer\0")
                .ok()
                .map(|s| *s);
            (init, get_delegate, close, set_writer)
        };
        if let Some(set_writer) = set_writer {
            // SAFETY: `error_writer` copies the message and returns; hostfxr
            // calls it on the thread that is calling hostfxr.
            unsafe { set_writer(Some(error_writer)) };
        }

        let config = host_str(runtime_config.as_os_str());
        let root = host_str(dotnet.root.as_os_str());
        let params = InitParams {
            size: std::mem::size_of::<InitParams>(),
            host_path: std::ptr::null(),
            dotnet_root: root.as_ptr(),
        };
        let mut handle: *mut c_void = std::ptr::null_mut();
        take_host_errors();
        // SAFETY: both strings are NUL-terminated and outlive the call.
        let rc = unsafe { init(config.as_ptr(), &params, &mut handle) };
        // 0 is Success, 1 Success_HostAlreadyInitialized, 2
        // Success_DifferentRuntimeProperties: all three mean a usable runtime.
        if rc < 0 || handle.is_null() {
            return Err(failure("starting the .NET runtime", rc));
        }
        let mut delegate: *mut c_void = std::ptr::null_mut();
        // SAFETY: `handle` was just returned by a successful initialise.
        let rc = unsafe { get_delegate(handle, HDT_LOAD_ASSEMBLY_AND_GET_FUNCTION_POINTER, &mut delegate) };
        // The context handle is only needed to get the delegate; the runtime
        // itself stays up after it is closed.
        // SAFETY: closing the handle we were given, once.
        unsafe { close(handle) };
        if rc < 0 || delegate.is_null() {
            return Err(failure("getting the .NET loader", rc));
        }
        Ok(Self {
            // SAFETY: the delegate for that `hdt_` value has this signature.
            load: unsafe { std::mem::transmute::<*mut c_void, LoadAssemblyAndGetFnFn>(delegate) },
        })
    }

    /// The `[UnmanagedCallersOnly]` static method `method` of `type_name`
    /// (assembly-qualified) in the assembly at `assembly`.
    ///
    /// The runtime loads the assembly into an isolated load context of its
    /// own the first time and reuses it after, so fetching several methods of
    /// one assembly loads it once.
    pub fn function(&self, assembly: &Path, type_name: &str, method: &str) -> Result<*mut c_void, String> {
        let asm = host_str(assembly.as_os_str());
        let ty = host_str(std::ffi::OsStr::new(type_name));
        let m = host_str(std::ffi::OsStr::new(method));
        let mut out: *mut c_void = std::ptr::null_mut();
        take_host_errors();
        // SAFETY: all strings are NUL-terminated and outlive the call; the
        // delegate type sentinel says the target is `[UnmanagedCallersOnly]`.
        let rc = unsafe {
            (self.load)(
                asm.as_ptr(),
                ty.as_ptr(),
                m.as_ptr(),
                UNMANAGED_CALLERS_ONLY,
                std::ptr::null_mut(),
                &mut out,
            )
        };
        if rc < 0 || out.is_null() {
            return Err(failure(&format!("finding {type_name}.{method}"), rc));
        }
        Ok(out)
    }
}
