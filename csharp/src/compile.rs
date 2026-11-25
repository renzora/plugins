//! Compiling C#: the plugin's own host assembly, and a project's scripts.
//!
//! Both are ordinary `dotnet build`s of a generated or shipped `.csproj`, run as
//! a child process off the main thread. Nothing here is a compiler of its own:
//! the SDK's is the one every C# developer already trusts, its error messages
//! are the ones they already know, and it needs no network unless the project
//! asks for NuGet packages (see [`read_packages`]): the reference assemblies
//! come with the SDK, and the host references no package at all.
//!
//! # Outputs are named by a hash of their inputs
//!
//! The host builds into `build/csharp/host-<hash>/` beside the plugin, the
//! scripts into `<project>/.renzora/csharp/bin/<hash>/`, where the hash covers
//! every source byte (and, for scripts, the host they compiled against). A
//! finished output gets a `.ok` marker, and an output with one is never built
//! again. That is what lets the editor and the Play process (a second process
//! loading this same plugin) share one build: whichever gets there first builds,
//! and the other finds the marker. It also makes "undo the edit" free: the
//! previous output is still there.
//!
//! # Output goes to a file, not a pipe
//!
//! `dotnet build` leaves helpers running after it exits (MSBuild nodes and the
//! compiler server, which is what makes the second build fast), and they inherit
//! the standard handles. Reading the build's output through a pipe would wait
//! for every one of them to exit, which is minutes. So the console logger is off
//! and the diagnostics go to a log file through MSBuild's file logger; the build
//! is waited on by exit status alone.
//!
//! This module depends on `std` alone, so the stand-alone test harness includes
//! it with `#[path]`.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

use crate::dotnet::{Dotnet, INSTALL_HINT};

/// The host assembly's file name, which `Bridge` is fetched from.
pub const HOST_DLL: &str = "Renzora.Scripting.dll";
const HOST_PROJECT: &str = "Renzora.Scripting.csproj";
/// The scripts assembly's name. The same in every project and every build:
/// each build is loaded into a fresh load context, where names never collide.
pub const SCRIPTS_ASSEMBLY: &str = "RenzoraScripts";
/// Markers that an output directory is complete.
const DONE: &str = ".ok";

/// How long a `dotnet build` may run before it is assumed stuck. The first
/// build on a fresh SDK unpacks its workload manifests and can take a minute;
/// nothing legitimate takes five.
const BUILD_TIMEOUT: Duration = Duration::from_secs(300);
/// A build lock older than this belongs to a process that died mid-build.
const STALE_LOCK: Duration = Duration::from_secs(600);

// =============================================================================
// Hashing
// =============================================================================

/// FNV-1a, 64-bit. Not `DefaultHasher`, whose output Rust reserves the right to
/// change between releases: these hashes name directories on disk, which two
/// differently built processes must agree on.
#[derive(Clone, Copy)]
pub struct Fnv(u64);

impl Default for Fnv {
    fn default() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}

impl Fnv {
    pub fn bytes(&mut self, bytes: &[u8]) -> &mut Self {
        for b in bytes {
            self.0 ^= *b as u64;
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
        self
    }

    /// A length-prefixed field, so `("ab", "c")` and `("a", "bc")` differ.
    pub fn field(&mut self, bytes: &[u8]) -> &mut Self {
        self.bytes(&(bytes.len() as u64).to_le_bytes()).bytes(bytes)
    }

    pub fn finish(&self) -> u64 {
        self.0
    }
}

pub fn hex(h: u64) -> String {
    format!("{h:016x}")
}

// =============================================================================
// Finding sources
// =============================================================================

/// Folders a source walk never enters: build output, and anything hidden
/// (`.git`, `.vs`).
fn skipped_dir(name: &str) -> bool {
    name.starts_with('.') || name.eq_ignore_ascii_case("bin") || name.eq_ignore_ascii_case("obj")
}

/// Every file under `dir` with one of `extensions`, sorted, so the order (and
/// so every hash over it) is the same on every machine.
pub fn find_files(dir: &Path, extensions: &[&str]) -> Vec<PathBuf> {
    fn walk(dir: &Path, extensions: &[&str], out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if path.is_dir() {
                if !skipped_dir(&name) {
                    walk(&path, extensions, out);
                }
            } else if path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| extensions.iter().any(|x| e.eq_ignore_ascii_case(x)))
            {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, extensions, &mut out);
    out.sort();
    out
}

/// A cheap fingerprint of `files`: their paths, sizes and modification times.
///
/// What the poll compares every half second to notice an edit, without reading
/// a byte of source. The content hash that names the output is computed only
/// once this moves, so touching a file without changing it costs one hash and
/// no build.
pub fn fingerprint(files: &[PathBuf]) -> u64 {
    let mut h = Fnv::default();
    for f in files {
        h.field(f.to_string_lossy().as_bytes());
        if let Ok(meta) = std::fs::metadata(f) {
            h.bytes(&meta.len().to_le_bytes());
            let t = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0);
            h.bytes(&t.to_le_bytes());
        }
    }
    h.finish()
}

/// A hash of every file's path (relative to `base`) and contents.
fn content_hash(base: &Path, files: &[PathBuf], salt: &[u8]) -> Result<u64, String> {
    let mut h = Fnv::default();
    h.field(salt);
    for f in files {
        let rel = f.strip_prefix(base).unwrap_or(f);
        h.field(rel.to_string_lossy().replace('\\', "/").as_bytes());
        let bytes = std::fs::read(f).map_err(|e| format!("could not read {}: {e}", f.display()))?;
        h.field(&bytes);
    }
    Ok(h.finish())
}

// =============================================================================
// Diagnostics
// =============================================================================

/// One compiler message.
#[derive(Clone, Debug, PartialEq)]
pub struct Diagnostic {
    /// The source file, as the compiler named it (a full path, since the build
    /// asks for `GenerateFullPaths`). `None` for a message about the project.
    pub file: Option<PathBuf>,
    pub line: u32,
    pub column: u32,
    pub error: bool,
    /// `CS1002: ; expected`.
    pub message: String,
}

impl Diagnostic {
    /// `spinner.cs(12,5): error CS1002: ; expected`, the way the compiler
    /// prints it but with the file name only: the author knows where their
    /// scripts folder is, and a full path wraps in the console.
    pub fn short(&self) -> String {
        let kind = if self.error { "error" } else { "warning" };
        match &self.file {
            Some(f) => format!(
                "{}({},{}): {kind} {}",
                f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                self.line,
                self.column,
                self.message
            ),
            None => format!("{kind} {}", self.message),
        }
    }
}

/// Parse MSBuild's canonical error format:
///
/// ```text
/// C:\p\scripts\spinner.cs(12,5): error CS1002: ; expected [C:\p\.renzora\csharp\Scripts.csproj]
/// CSC : error CS2001: Source file 'x.cs' could not be found. [...]
/// ```
///
/// Lines in any other shape (the file logger's header, timing) are not
/// diagnostics and are skipped. Duplicates are dropped: MSBuild repeats a
/// message when two targets report it.
pub fn parse_diagnostics(log: &str) -> Vec<Diagnostic> {
    let mut out: Vec<Diagnostic> = Vec::new();
    for raw in log.lines() {
        let line = raw.trim();
        let (head, kind, rest) = if let Some(i) = line.find(": error ") {
            (&line[..i], true, &line[i + ": error ".len()..])
        } else if let Some(i) = line.find(": warning ") {
            (&line[..i], false, &line[i + ": warning ".len()..])
        } else {
            continue;
        };
        // The trailing ` [project]` is MSBuild's, not the compiler's.
        let message = match rest.rfind(" [") {
            Some(i) if rest.ends_with(']') => &rest[..i],
            _ => rest,
        }
        .trim()
        .to_string();
        let (file, line_no, col) = match head.rfind('(') {
            Some(open) if head.ends_with(')') => {
                let pos = &head[open + 1..head.len() - 1];
                let mut nums = pos.split(',').map(|n| n.trim().parse::<u32>().unwrap_or(0));
                let l = nums.next().unwrap_or(0);
                let c = nums.next().unwrap_or(0);
                (Some(PathBuf::from(head[..open].trim())), l, c)
            }
            _ => (None, 0, 0),
        };
        let d = Diagnostic {
            file,
            line: line_no,
            column: col,
            error: kind,
            message,
        };
        if !out.contains(&d) {
            out.push(d);
        }
    }
    out
}

// =============================================================================
// Running dotnet
// =============================================================================

/// `dotnet build <project> -c Release -o <out>` with the diagnostics written to
/// `log`. Returns whether it succeeded and the diagnostics.
fn dotnet_build(dotnet: &Dotnet, project: &Path, out: &Path, log: &Path) -> Result<(bool, Vec<Diagnostic>), String> {
    let _ = std::fs::remove_file(log);
    let mut cmd = Command::new(&dotnet.exe);
    cmd.arg("build")
        .arg(project)
        .args(["-c", "Release", "-o"])
        .arg(out)
        .args(["-nologo", "-noConsoleLogger", "-p:GenerateFullPaths=true"])
        .arg(format!("-flp:LogFile={};Verbosity=quiet;NoSummary;Encoding=UTF-8", log.display()))
        .current_dir(project.parent().unwrap_or(Path::new(".")))
        // The same .NET for the build as for the runtime, and none of the
        // first-run chatter a fresh SDK otherwise does in a hidden window.
        .env("DOTNET_ROOT", &dotnet.root)
        .env("DOTNET_MULTILEVEL_LOOKUP", "0")
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .env("DOTNET_NOLOGO", "1")
        .env("DOTNET_SKIP_FIRST_TIME_EXPERIENCE", "1")
        .env("DOTNET_GENERATE_ASPNET_CERTIFICATE", "false")
        .env("MSBUILDTERMINALLOGGER", "off")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: the editor is a windowed process, and without this
        // every build flashes a console window over it.
        cmd.creation_flags(0x0800_0000);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("could not run {}: {e}", dotnet.exe.display()))?;
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() > BUILD_TIMEOUT => {
                let _ = child.kill();
                return Err(format!("dotnet build took longer than {}s and was stopped", BUILD_TIMEOUT.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(format!("dotnet build: {e}")),
        }
    };
    let text = std::fs::read(log).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
    let diagnostics = parse_diagnostics(&text);
    Ok((status.success(), diagnostics))
}

/// Write `contents` to `path` unless it already holds exactly that, so an
/// unchanged generated project keeps its timestamp and MSBuild's incremental
/// checks keep working.
fn write_if_changed(path: &Path, contents: &str) -> Result<(), String> {
    if std::fs::read_to_string(path).is_ok_and(|old| old == contents) {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    }
    std::fs::write(path, contents).map_err(|e| format!("could not write {}: {e}", path.display()))
}

/// A lock on one build directory across processes, released on drop.
///
/// The editor and its Play process can both notice the same edit at once; two
/// `dotnet build`s sharing one `obj/` corrupt each other's intermediate files.
/// So the second waits for the first, then usually finds its `.ok` marker.
struct BuildLock(PathBuf);

impl BuildLock {
    fn acquire(path: PathBuf, done: &Path) -> Result<Option<Self>, String> {
        let started = Instant::now();
        loop {
            match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(_) => return Ok(Some(Self(path))),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    // The other process finished what we came to do.
                    if done.is_file() {
                        return Ok(None);
                    }
                    let age = std::fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.elapsed().ok())
                        .unwrap_or_default();
                    if age > STALE_LOCK {
                        let _ = std::fs::remove_file(&path);
                        continue;
                    }
                    if started.elapsed() > BUILD_TIMEOUT {
                        return Err(format!("another process has been building in {} for too long", path.display()));
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(e) => return Err(format!("could not create {}: {e}", path.display())),
            }
        }
    }
}

impl Drop for BuildLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Remove every `<prefix>*` directory in `dir` but `keep`, best effort. A
/// directory another process still has an assembly loaded from is locked on
/// Windows and simply stays until a later build.
fn prune(dir: &Path, prefix: &str, keep: &[&Path]) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() && name.starts_with(prefix) && !keep.iter().any(|k| *k == path) {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

// =============================================================================
// The host
// =============================================================================

/// The built host assembly.
#[derive(Clone, Debug)]
pub struct Host {
    pub dll: PathBuf,
    pub runtime_config: PathBuf,
    /// The hash of its sources, which every scripts build folds in: scripts
    /// compiled against one host must not be loaded beside another.
    pub hash: u64,
}

/// Build the host assembly from `<plugin_dir>/host/`, or reuse the build of
/// the same sources. `salt` is folded into the hash (the ABI version), so a
/// plugin update that changes the protocol never loads an old host.
///
/// The sources are copied into `build/csharp/host-src/` and built there, so
/// the build's `obj/` never lands in the shipped source folder.
pub fn ensure_host(dotnet: &Dotnet, plugin_dir: &Path, salt: &[u8]) -> Result<Host, String> {
    let src = plugin_dir.join("host");
    let files = find_files(&src, &["cs", "csproj"]);
    if !files.iter().any(|f| f.file_name().is_some_and(|n| n == HOST_PROJECT)) {
        return Err(format!(
            "the plugin's C# host is missing ({} not found); reinstall the plugin",
            src.join(HOST_PROJECT).display()
        ));
    }
    let hash = content_hash(&src, &files, salt)?;
    let root = plugin_dir.join("build").join("csharp");
    let out = root.join(format!("host-{}", hex(hash)));
    let host = Host {
        dll: out.join(HOST_DLL),
        runtime_config: out.join("Renzora.Scripting.runtimeconfig.json"),
        hash,
    };
    if out.join(DONE).is_file() && host.dll.is_file() {
        return Ok(host);
    }
    if !dotnet.has_sdk {
        return Err(format!(
            "the C# host has not been built on this machine and no .NET SDK is installed to build it; {INSTALL_HINT}"
        ));
    }
    std::fs::create_dir_all(&root).map_err(|e| format!("could not create {}: {e}", root.display()))?;
    let _lock = match BuildLock::acquire(root.join("host.lock"), &out.join(DONE))? {
        Some(lock) => lock,
        None => return Ok(host),
    };

    let work = root.join("host-src");
    // Replace the previous copy's sources (a file removed from the plugin must
    // not linger in the build), keeping its `obj/` for incremental builds.
    for old in find_files(&work, &["cs", "csproj"]) {
        let _ = std::fs::remove_file(old);
    }
    for f in &files {
        let rel = f.strip_prefix(&src).unwrap_or(f);
        let dest = work.join(rel);
        if let Some(dir) = dest.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
        }
        std::fs::copy(f, &dest).map_err(|e| format!("could not copy {}: {e}", f.display()))?;
    }

    let (ok, diagnostics) = dotnet_build(dotnet, &work.join(HOST_PROJECT), &out, &root.join("host-build.log"))?;
    if !ok || !host.dll.is_file() {
        let errors: Vec<String> = diagnostics.iter().filter(|d| d.error).map(Diagnostic::short).collect();
        return Err(if errors.is_empty() {
            "the plugin's C# host did not build, and dotnet gave no reason".to_string()
        } else {
            format!("the plugin's C# host did not build: {}", errors.join("; "))
        });
    }
    if !host.runtime_config.is_file() {
        // `EnableDynamicLoading` writes this; a host project edited without it
        // would otherwise fail later with a far vaguer message.
        write_runtime_config(&host.runtime_config)?;
    }
    std::fs::write(out.join(DONE), b"").map_err(|e| format!("could not finish {}: {e}", out.display()))?;
    prune(&root, "host-", &[&out, &work]);
    Ok(host)
}

/// The runtime the host asks for: .NET 10, or any newer major version when 10
/// is not installed (`LatestMajor`), since nothing in it is version specific.
fn write_runtime_config(path: &Path) -> Result<(), String> {
    write_if_changed(
        path,
        r#"{
  "runtimeOptions": {
    "tfm": "net10.0",
    "rollForward": "LatestMajor",
    "framework": { "name": "Microsoft.NETCore.App", "version": "10.0.0" }
  }
}
"#,
    )
}

// =============================================================================
// A project's scripts
// =============================================================================

/// A scripts build that failed, with every diagnostic it produced.
#[derive(Clone, Debug, Default)]
pub struct Failure {
    pub diagnostics: Vec<Diagnostic>,
    /// A failure with no diagnostic: dotnet could not run, a file could not be
    /// written.
    pub message: Option<String>,
}

/// A successful scripts build.
#[derive(Clone, Debug)]
pub struct Built {
    pub dll: PathBuf,
    /// Warnings, which do not stop anything but deserve a console line.
    pub warnings: Vec<Diagnostic>,
    /// Whether this call compiled (false when an earlier build was reused).
    pub compiled: bool,
}

/// The project a scripts folder belongs to: the folder that holds
/// `project.toml`. The engine hands backends the project root itself, so the
/// folder is usually the answer; a `scripts/` folder inside a project is
/// recognised too. When neither has one (a bare folder in a test), the folder
/// stands in for the project.
pub fn project_root(scripts_folder: &Path) -> PathBuf {
    if scripts_folder.join("project.toml").is_file() {
        return scripts_folder.to_path_buf();
    }
    match scripts_folder.parent() {
        Some(parent) if parent.join("project.toml").is_file() => parent.to_path_buf(),
        _ => scripts_folder.to_path_buf(),
    }
}

/// Where a project's C# build lives: `<project>/.renzora/csharp/`.
///
/// Not `<scripts folder>/..`: the engine's scripts folder is the project root,
/// and its parent put every project's build beside the project, in one shared
/// folder of the user's documents.
pub fn project_dir(scripts_folder: &Path) -> PathBuf {
    project_root(scripts_folder).join(".renzora").join("csharp")
}

// =============================================================================
// NuGet packages
// =============================================================================

/// The file a project lists its NuGet packages in, at the project root.
pub const PACKAGES_FILE: &str = "csharp.toml";

/// One NuGet package a project's scripts reference.
#[derive(Clone, Debug, PartialEq)]
pub struct Package {
    pub name: String,
    pub version: String,
}

/// `<project>/csharp.toml`, if the project has one.
pub fn packages_file(scripts_folder: &Path) -> Option<PathBuf> {
    let path = project_root(scripts_folder).join(PACKAGES_FILE);
    path.is_file().then_some(path)
}

/// The packages `<project>/csharp.toml` lists, empty when there is no file.
///
/// ```toml
/// [packages]
/// "Newtonsoft.Json" = "13.0.3"
/// Humanizer.Core = "2.14.1"
/// ```
///
/// Read by hand rather than with a TOML crate: the file is one table of
/// name-version pairs, this module is `std` only (the test harness includes
/// it), and a package name is dotted, which real TOML would read as nested
/// tables unless quoted. Both spellings are accepted here, since forgetting the
/// quotes is the mistake everyone makes once.
pub fn read_packages(scripts_folder: &Path) -> Result<Vec<Package>, String> {
    let Some(path) = packages_file(scripts_folder) else {
        return Ok(Vec::new());
    };
    let text = std::fs::read_to_string(&path).map_err(|e| format!("could not read {}: {e}", path.display()))?;
    parse_packages(&text).map_err(|e| format!("{PACKAGES_FILE}: {e}"))
}

pub fn parse_packages(text: &str) -> Result<Vec<Package>, String> {
    let mut out: Vec<Package> = Vec::new();
    let mut in_packages = false;
    for (n, raw) in text.lines().enumerate() {
        let line_no = n + 1;
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            in_packages = line == "[packages]";
            continue;
        }
        if !in_packages {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return Err(format!("line {line_no}: expected `\"Package.Name\" = \"1.2.3\"`"));
        };
        let name = unquote(key.trim());
        let version = unquote(value.trim());
        // Checked, not escaped: both go into the generated project's XML, and
        // nothing a real package name or version range holds needs escaping.
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')) {
            return Err(format!("line {line_no}: `{name}` is not a NuGet package name"));
        }
        if version.is_empty()
            || !version
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+' | '*' | '[' | ']' | '(' | ')' | ',' | ' '))
        {
            return Err(format!("line {line_no}: `{version}` is not a version for {name} (write one such as \"13.0.3\")"));
        }
        if out.iter().any(|p| p.name.eq_ignore_ascii_case(&name)) {
            return Err(format!("line {line_no}: {name} is listed twice"));
        }
        out.push(Package { name, version });
    }
    Ok(out)
}

/// A line without its `#` comment, ignoring a `#` inside quotes.
fn strip_comment(line: &str) -> &str {
    let mut quoted = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '#' if !quoted => return &line[..i],
            _ => {}
        }
    }
    line
}

fn unquote(s: &str) -> String {
    s.strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| s.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
        .unwrap_or(s)
        .to_string()
}

/// The generated project that compiles every `.cs` under `scripts_folder`
/// into one assembly against `host`.
///
/// The scripts folder is named by its absolute path, so the generated file is
/// right wherever `.renzora` is; it is rewritten whenever that changes.
/// Implicit usings are on (`System`, `System.Linq`, `System.Collections.Generic`
/// and the rest of the SDK's list), so a script starts with `using Renzora;`
/// and nothing else.
///
/// Packages are copied into the output (`CopyLocalLockFileAssemblies`: a
/// library does not get its packages' assemblies otherwise) with a
/// `.deps.json` beside them, which is how the load context finds a package's
/// assemblies and its native libraries for this platform.
fn scripts_project(scripts_folder: &Path, host: &Host, packages: &[Package]) -> String {
    let scripts = scripts_folder.display().to_string();
    let sep = if scripts.ends_with('\\') || scripts.ends_with('/') { "" } else { "/" };
    let package_items: String = packages
        .iter()
        .map(|p| format!("    <PackageReference Include=\"{}\" Version=\"{}\" />\n", p.name, p.version))
        .collect();
    format!(
        r#"<!-- Generated by the C# Scripting plugin on every build; edits are overwritten.
     Open it in an IDE for completion against the Renzora API. -->
<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <TargetFramework>net10.0</TargetFramework>
    <AssemblyName>{SCRIPTS_ASSEMBLY}</AssemblyName>
    <RootNamespace>Scripts</RootNamespace>
    <Nullable>enable</Nullable>
    <ImplicitUsings>enable</ImplicitUsings>
    <AllowUnsafeBlocks>true</AllowUnsafeBlocks>
    <EnableDefaultCompileItems>false</EnableDefaultCompileItems>
    <GenerateAssemblyInfo>false</GenerateAssemblyInfo>
    <GenerateDependencyFile>true</GenerateDependencyFile>
    <CopyLocalLockFileAssemblies>true</CopyLocalLockFileAssemblies>
    <AppendTargetFrameworkToOutputPath>false</AppendTargetFrameworkToOutputPath>
    <SatelliteResourceLanguages>en</SatelliteResourceLanguages>
    <DebugType>portable</DebugType>
    <Deterministic>true</Deterministic>
  </PropertyGroup>
  <ItemGroup>
    <Compile Include="{scripts}{sep}**/*.cs" Exclude="{scripts}{sep}**/bin/**;{scripts}{sep}**/obj/**" />
    <Reference Include="Renzora.Scripting">
      <HintPath>{host}</HintPath>
      <Private>false</Private>
    </Reference>
{package_items}  </ItemGroup>
</Project>
"#,
        host = host.dll.display(),
    )
}

/// Compile `files` (every `.cs` under `scripts_folder`) or reuse the build of
/// the same sources.
pub fn ensure_scripts(dotnet: &Dotnet, host: &Host, scripts_folder: &Path, files: &[PathBuf]) -> Result<Built, Failure> {
    let fail = |message: String| Failure {
        diagnostics: Vec::new(),
        message: Some(message),
    };
    let packages = read_packages(scripts_folder).map_err(fail)?;
    // The package list is part of what the output is built from: adding one
    // to `csharp.toml` must make a new build even with no `.cs` changed.
    let mut salt = Fnv::default();
    salt.bytes(&host.hash.to_le_bytes());
    for p in &packages {
        salt.field(p.name.to_ascii_lowercase().as_bytes()).field(p.version.as_bytes());
    }
    let hash = content_hash(scripts_folder, files, &salt.finish().to_le_bytes()).map_err(fail)?;
    let dir = project_dir(scripts_folder);
    let bin = dir.join("bin");
    let out = bin.join(hex(hash));
    let dll = out.join(format!("{SCRIPTS_ASSEMBLY}.dll"));
    let done = out.join(DONE);
    if done.is_file() && dll.is_file() {
        return Ok(Built {
            dll,
            warnings: Vec::new(),
            compiled: false,
        });
    }
    if !dotnet.has_sdk {
        return Err(fail(format!(
            "these C# scripts have not been built on this machine and no .NET SDK is installed to build them; {INSTALL_HINT}"
        )));
    }
    std::fs::create_dir_all(&dir).map_err(|e| fail(format!("could not create {}: {e}", dir.display())))?;
    let _lock = match BuildLock::acquire(dir.join("build.lock"), &done).map_err(fail)? {
        Some(lock) => lock,
        None => {
            return Ok(Built {
                dll,
                warnings: Vec::new(),
                compiled: false,
            })
        }
    };
    let project = dir.join("Scripts.csproj");
    write_if_changed(&project, &scripts_project(scripts_folder, host, &packages)).map_err(fail)?;
    // A stray `.gitignore` here keeps a project's repository from picking up
    // build output nobody meant to commit.
    let _ = write_if_changed(&dir.join(".gitignore"), "bin/\nobj/\n*.log\nbuild.lock\n");

    let (ok, diagnostics) = dotnet_build(dotnet, &project, &out, &dir.join("build.log")).map_err(fail)?;
    if !ok || !dll.is_file() {
        let mut failure = Failure {
            diagnostics: diagnostics.into_iter().filter(|d| d.error).collect(),
            message: None,
        };
        if failure.diagnostics.is_empty() {
            failure.message = Some(format!(
                "dotnet build failed without a compiler error; see {}",
                dir.join("build.log").display()
            ));
        }
        // The failed output folder holds nothing worth keeping.
        let _ = std::fs::remove_dir_all(&out);
        return Err(failure);
    }
    std::fs::write(&done, b"").map_err(|e| fail(format!("could not finish {}: {e}", out.display())))?;
    keep_recent(&bin, &out, 4);
    Ok(Built {
        dll,
        warnings: diagnostics.into_iter().filter(|d| !d.error).collect(),
        compiled: true,
    })
}

// =============================================================================
// An exported game's build
// =============================================================================

/// Where an export puts the C# build, inside the plugin's folder in the game.
pub const EXPORT_DIR: &str = "export";
const EXPORT_MANIFEST: &str = "manifest.txt";

/// The C# build an export shipped beside the plugin: the host, the scripts and
/// their packages, ready to load. A game has no SDK and no sources, so it runs
/// exactly what the editor built.
#[derive(Clone, Debug)]
pub struct Exported {
    pub host: Host,
    pub scripts: PathBuf,
    /// The project folder the scripts were compiled in. The build names each
    /// script by its full path there (the PDB records it); the game rebases
    /// those onto its own folder.
    pub built_in: String,
    /// The .NET runtime the export bundled, laid out as a .NET root, when it
    /// bundled one. `None` means the player's own .NET is used.
    pub dotnet_root: Option<PathBuf>,
}

/// Where in the export a bundled runtime goes: `export/dotnet/`, laid out as a
/// .NET root (`host/fxr/`, `shared/Microsoft.NETCore.App/`).
pub fn export_runtime_path(rel_in_root: &str) -> String {
    format!("{EXPORT_DIR}/dotnet/{rel_in_root}")
}

/// The manifest an export writes with the build. `build` names this build's
/// contents: a game unpacks a build once per distinct `build`.
pub fn export_manifest(project_root: &Path, build: u64) -> String {
    format!(
        "# The C# build this game ships; written by the export.\nbuilt_in={}\nbuild={}\n",
        project_root.display(),
        hex(build)
    )
}

/// An id for a set of files to ship: each path in the build, its size and
/// its modification time. Cheap where a content hash of a 76 MB runtime is
/// not, and a rebuilt file always changes at least the time.
pub fn build_id(files: &[(String, PathBuf)]) -> u64 {
    let mut h = Fnv::default();
    for (rel, src) in files {
        h.field(rel.as_bytes());
        if let Ok(meta) = std::fs::metadata(src) {
            h.bytes(&meta.len().to_le_bytes());
            let t = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0);
            h.bytes(&t.to_le_bytes());
        }
    }
    h.finish()
}

/// The exported build in `plugin_dir`, if this is a game that has one.
pub fn exported_build(plugin_dir: &Path) -> Option<Exported> {
    let dir = plugin_dir.join(EXPORT_DIR);
    let manifest = std::fs::read_to_string(dir.join(EXPORT_MANIFEST)).ok()?;
    let built_in = manifest.lines().find_map(|l| l.strip_prefix("built_in=")).unwrap_or("").trim().to_string();
    let host_dir = dir.join("host");
    let host = Host {
        dll: host_dir.join(HOST_DLL),
        runtime_config: host_dir.join("Renzora.Scripting.runtimeconfig.json"),
        hash: 0,
    };
    let scripts = dir.join("scripts").join(format!("{SCRIPTS_ASSEMBLY}.dll"));
    let dotnet = dir.join("dotnet");
    let dotnet_root = dotnet.join("host").join("fxr").is_dir().then_some(dotnet);
    (host.dll.is_file() && host.runtime_config.is_file() && scripts.is_file()).then_some(Exported {
        host,
        scripts,
        built_in,
        dotnet_root,
    })
}

/// Paths of the export: `(path in the plugin's folder, file on this machine)`
/// for the host, the scripts build and the manifest's name.
pub fn export_layout(host: &Host, scripts_dll: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    for (to, dir) in [("host", host.dll.parent()), ("scripts", scripts_dll.parent())] {
        let Some(dir) = dir else { continue };
        for f in list_dir(dir) {
            // The build's completion marker means nothing in a game.
            if f.file_name().is_some_and(|n| n == DONE) {
                continue;
            }
            let rel = f.strip_prefix(dir).unwrap_or(&f).to_string_lossy().replace('\\', "/");
            out.push((format!("{EXPORT_DIR}/{to}/{rel}"), f));
        }
    }
    out
}

/// Every file under `dir`, sorted.
fn list_dir(dir: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, &mut out);
    out.sort();
    out
}

pub fn export_manifest_path() -> String {
    format!("{EXPORT_DIR}/{EXPORT_MANIFEST}")
}

/// Keep the `count` most recent finished builds in `bin` (and always `keep`),
/// so a session of edits does not leave hundreds of folders, while an undo to
/// the last few versions still costs no build.
fn keep_recent(bin: &Path, keep: &Path, count: usize) {
    let Ok(entries) = std::fs::read_dir(bin) else {
        return;
    };
    let mut dirs: Vec<(SystemTime, PathBuf)> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p != keep)
        .map(|p| {
            let t = std::fs::metadata(p.join(DONE))
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            (t, p)
        })
        .collect();
    dirs.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, old) in dirs.into_iter().skip(count.saturating_sub(1)) {
        let _ = std::fs::remove_dir_all(old);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packages_parse_quoted_bare_and_commented() {
        let text = r#"
# NuGet packages for this project's C# scripts
[other]
"Ignored" = "1.0"
[packages]
"Newtonsoft.Json" = "13.0.3"   # quoted
Humanizer.Core = '2.14.1'
"#;
        let p = parse_packages(text).unwrap();
        assert_eq!(
            p,
            vec![
                Package { name: "Newtonsoft.Json".into(), version: "13.0.3".into() },
                Package { name: "Humanizer.Core".into(), version: "2.14.1".into() },
            ]
        );
    }

    #[test]
    fn a_package_that_would_break_the_project_is_refused() {
        assert!(parse_packages("[packages]\n\"A\" = \"1\\\" /><Evil\"").is_err());
        assert!(parse_packages("[packages]\n\"<A>\" = \"1.0\"").is_err());
        assert!(parse_packages("[packages]\nA = \"1.0\"\na = \"2.0\"").is_err());
        assert!(parse_packages("[packages]\nNoVersion").is_err());
    }

    #[test]
    fn a_compiler_error_parses_with_its_position() {
        let log = r"C:\p\scripts\spinner.cs(12,5): error CS1002: ; expected [C:\p\.renzora\csharp\Scripts.csproj]";
        let d = parse_diagnostics(log);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].file.as_deref(), Some(Path::new(r"C:\p\scripts\spinner.cs")));
        assert_eq!((d[0].line, d[0].column), (12, 5));
        assert!(d[0].error);
        assert_eq!(d[0].message, "CS1002: ; expected");
        assert_eq!(d[0].short(), "spinner.cs(12,5): error CS1002: ; expected");
    }

    #[test]
    fn a_project_level_error_has_no_file() {
        let d = parse_diagnostics("CSC : error CS2001: Source file 'x.cs' could not be found. [a.csproj]");
        assert_eq!(d.len(), 1);
        assert!(d[0].file.is_none());
    }

    #[test]
    fn repeated_messages_are_reported_once() {
        let line = r"C:\a.cs(1,1): warning CS0168: The variable 'x' is declared but never used [p.csproj]";
        assert_eq!(parse_diagnostics(&format!("{line}\n{line}\n")).len(), 1);
    }
}
