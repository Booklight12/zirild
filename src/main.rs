//! cargo-zirild: build a Cargo project through Zig's C/C++ and linker tools.

use std::{
    env,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

const USAGE: &str = r#"cargo-zirild - run Cargo commands with the Zig toolchain

Usage:
  cargo zirild -target=<zig-target> [options] [cargo-command] [cargo options]

Required:
  -target=<zig-target>        Zig target, e.g. x86_64-linux-musl

Zig optimisation:
  (default/dev)               ReleaseSafe
  --release                   ReleaseFast
  -Z                          ReleaseSmall
  -ZigOptimize=<mode>         Explicit: Debug, ReleaseSafe, ReleaseFast, ReleaseSmall

Other options:
  -zigpatch=<path>            Use this Zig executable/directory before Zig_home
  -ndkversion=<version>       Android: select an exact NDK version below Ndk_home
  -nindx=<index>              Android: select NDK by newest-first index (0 = newest)
  -android-api=<level>        Android: use an Android API level for Zig's target
  -android-abi=<abi>          Android: validate ABI (arm64-v8a, x86_64, etc.)
  -android-stl=<name>         Android: export the requested NDK STL setting
  -android-cflag=<flag>       Android: pass one flag to the selected native compiler
  -android-link-arg=<flag>    Android: pass one flag to the selected native linker
  -ndkfallback                Android: explicitly use NDK Clang/LLD instead of Zig
  -h, --help                  Show this help

Cargo command:
  build (default), check, run, test, bench, rustc, clippy, or doc

All remaining arguments are passed to the selected Cargo command, for example
--features, --package, --bin, --locked, and run arguments after --. Do not pass
Cargo's --target: -target is the single source of truth and is converted to
Cargo's corresponding Rust triple.

Zig location:
  Set Zig_home to the Zig installation directory or the full path to zig/zig.exe.

Android NDK location:
  Set Ndk_home to either an NDK installation or Android SDK's ndk directory
  containing version subdirectories. Zirild always keeps Zig as the final
  compiler/linker; it never silently replaces it with NDK clang. CMake-style
  -DANDROID_ABI=..., -DANDROID_PLATFORM=android-<api>, and -DANDROID_STL=...
  are also parsed in Android mode. -ndkfallback is the explicit LLVM fallback.
"#;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WrapperMode {
    Cc,
    Cxx,
    Linker,
    Ar,
    Dlltool,
}

#[derive(Debug)]
struct BuildOptions {
    zig_target: String,
    cargo_target: String,
    optimize: String,
    zig_patch: Option<PathBuf>,
    cargo_command: String,
    cargo_args: Vec<OsString>,
    android: AndroidOptions,
}

#[derive(Debug, Default)]
struct AndroidOptions {
    ndk_version: Option<String>,
    ndk_index: Option<usize>,
    api_level: Option<u32>,
    abi: Option<String>,
    stl: Option<String>,
    cflags: Vec<String>,
    link_args: Vec<String>,
    ndk_fallback: bool,
}

#[derive(Debug)]
struct AndroidNdk {
    root: PathBuf,
    version: String,
    sysroot: PathBuf,
    cc: PathBuf,
    cxx: PathBuf,
    ar: PathBuf,
}

fn main() -> ExitCode {
    if let Some(mode) = wrapper_mode() {
        return report(run_wrapper(mode));
    }
    report(run_driver(env::args_os().skip(1)))
}

fn report(result: Result<(), String>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("cargo-zirild: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run_driver(args: impl IntoIterator<Item = OsString>) -> Result<(), String> {
    let mut args: Vec<_> = args.into_iter().collect();
    // Cargo passes the external subcommand name as argv[1], so
    // cargo zirild ... arrives as cargo-zirild zirild ....
    if args.first().is_some_and(|argument| argument == "zirild") {
        args.remove(0);
    }
    if args
        .iter()
        .take_while(|argument| *argument != "--")
        .any(|argument| argument == "--help" || argument == "-h")
    {
        print!("{USAGE}");
        return Ok(());
    }

    let options = parse_args(args)?;
    let android_ndk = if is_android_target(&options.cargo_target) {
        Some(android_ndk(&options.android, &options.cargo_target)?)
    } else {
        if options.android.is_configured() {
            return Err("Android-only options require an Android -target".into());
        }
        None
    };
    let zig = if options.android.ndk_fallback {
        None
    } else {
        Some(zig_executable(options.zig_patch.as_deref())?)
    };
    warn_about_android_target(
        &options.zig_target,
        android_ndk.as_ref(),
        options.android.ndk_fallback,
    );
    let wrappers = make_wrappers()?;
    let result = run_cargo(&options, zig.as_deref(), &wrappers, android_ndk.as_ref());
    let _ = fs::remove_dir_all(&wrappers.directory);
    result
}

fn parse_args(args: Vec<OsString>) -> Result<BuildOptions, String> {
    let mut target = None;
    let mut explicit_optimize = None;
    let mut zig_patch = None;
    let mut release = false;
    let mut small = false;
    let mut cargo_command = None;
    let mut cargo_args = Vec::new();
    let mut android = AndroidOptions::default();
    let mut passthrough = false;
    let mut cargo_value_expected = false;

    for argument in args {
        let text = argument.to_string_lossy();
        if passthrough {
            cargo_args.push(argument);
            continue;
        }
        if cargo_value_expected {
            cargo_args.push(argument);
            cargo_value_expected = false;
            continue;
        }
        // Cargo may inject the external subcommand name before or after user
        // options, depending on the invoking Cargo version and platform.
        if argument == "zirild" && cargo_command.is_none() {
            continue;
        } else if let Some(value) = text.strip_prefix("-target=") {
            target = Some(value.to_owned());
        } else if let Some(value) = text.strip_prefix("--target=") {
            target = Some(value.to_owned());
        } else if let Some(value) = text.strip_prefix("-ZigOptimize=") {
            explicit_optimize = Some(validate_optimize(value)?);
        } else if let Some(value) = text.strip_prefix("-zigpatch=") {
            if value.is_empty() {
                return Err("-zigpatch cannot be empty".into());
            }
            zig_patch = Some(PathBuf::from(value));
        } else if let Some(value) = text.strip_prefix("-ndkversion=") {
            if value.is_empty() {
                return Err("-ndkversion cannot be empty".into());
            }
            android.ndk_version = Some(value.into());
        } else if let Some(value) = text.strip_prefix("-nindx=") {
            android.ndk_index = Some(value.parse().map_err(|_| {
                format!("invalid -nindx '{value}'; use a non-negative integer (0 = newest)")
            })?);
        } else if let Some(value) = text.strip_prefix("-android-api=") {
            set_android_api_level(&mut android, value)?;
        } else if let Some(value) = text.strip_prefix("-android-abi=") {
            if value.is_empty() {
                return Err("-android-abi cannot be empty".into());
            }
            android.abi = Some(value.into());
        } else if let Some(value) = text.strip_prefix("-android-stl=") {
            if value.is_empty() {
                return Err("-android-stl cannot be empty".into());
            }
            android.stl = Some(value.into());
        } else if let Some(value) = text.strip_prefix("-android-cflag=") {
            if value.is_empty() {
                return Err("-android-cflag cannot be empty".into());
            }
            android.cflags.push(value.into());
        } else if let Some(value) = text.strip_prefix("-android-link-arg=") {
            if value.is_empty() {
                return Err("-android-link-arg cannot be empty".into());
            }
            android.link_args.push(value.into());
        } else if argument == "-ndkfallback" {
            android.ndk_fallback = true;
        } else if let Some(value) = text.strip_prefix("-DANDROID_ABI=") {
            if value.is_empty() {
                return Err("-DANDROID_ABI cannot be empty".into());
            }
            android.abi = Some(value.into());
        } else if let Some(value) = text.strip_prefix("-DANDROID_PLATFORM=") {
            set_android_api_level(&mut android, value)?;
        } else if let Some(value) = text.strip_prefix("-DANDROID_STL=") {
            if value.is_empty() {
                return Err("-DANDROID_STL cannot be empty".into());
            }
            android.stl = Some(value.into());
        } else if argument == "--release" {
            release = true;
            cargo_args.push(argument);
        } else if argument == "-Z" {
            small = true;
        } else if argument == "--" {
            passthrough = true;
            cargo_args.push(argument);
        } else if argument == "--target" || argument == "-target" {
            return Err("use -target=<zig-target>, not a separate Cargo --target".into());
        } else if cargo_command.is_none()
            && let Some(command) = cargo_command_name(&text)
        {
            cargo_command = Some(command.to_owned());
        } else {
            cargo_value_expected = cargo_option_takes_value(&text);
            cargo_args.push(argument);
        }
    }

    let target = target.ok_or_else(|| "missing required -target=<zig-target>".to_owned())?;
    if target.is_empty() {
        return Err("-target cannot be empty".into());
    }
    let optimize = explicit_optimize.unwrap_or_else(|| {
        if small {
            "ReleaseSmall".into()
        } else if release {
            "ReleaseFast".into()
        } else {
            "ReleaseSafe".into()
        }
    });
    let cargo_target = cargo_target_for(&target);
    let mut zig_target = zig_target_for(&target);
    if is_android_target(&cargo_target) {
        validate_android_options(&android, &cargo_target)?;
        if let Some(api_level) = android.api_level {
            zig_target.push('.');
            zig_target.push_str(&api_level.to_string());
        }
    }

    Ok(BuildOptions {
        zig_target,
        cargo_target,
        optimize,
        zig_patch,
        cargo_command: cargo_command.unwrap_or_else(|| "build".into()),
        cargo_args,
        android,
    })
}

impl AndroidOptions {
    fn is_configured(&self) -> bool {
        self.ndk_version.is_some()
            || self.ndk_index.is_some()
            || self.api_level.is_some()
            || self.abi.is_some()
            || self.stl.is_some()
            || !self.cflags.is_empty()
            || !self.link_args.is_empty()
            || self.ndk_fallback
    }
}

fn set_android_api_level(android: &mut AndroidOptions, value: &str) -> Result<(), String> {
    let value = value.strip_prefix("android-").unwrap_or(value);
    let api_level = value
        .parse::<u32>()
        .map_err(|_| format!("invalid Android API level '{value}'"))?;
    if api_level == 0 {
        return Err("Android API level must be greater than zero".into());
    }
    if let Some(previous) = android.api_level
        && previous != api_level
    {
        return Err(format!(
            "conflicting Android API levels: android-{previous} and android-{api_level}"
        ));
    }
    android.api_level = Some(api_level);
    Ok(())
}

fn validate_android_options(android: &AndroidOptions, cargo_target: &str) -> Result<(), String> {
    if android.ndk_version.is_some() && android.ndk_index.is_some() {
        return Err("use either -ndkversion or -nindx, not both".into());
    }
    if let Some(abi) = &android.abi
        && let Some(expected) = android_abi_for_target(cargo_target)
        && abi != expected
    {
        return Err(format!(
            "Android ABI '{abi}' does not match target {cargo_target}; use {expected}"
        ));
    }
    Ok(())
}

fn cargo_command_name(argument: &str) -> Option<&'static str> {
    match argument {
        "build" => Some("build"),
        "check" => Some("check"),
        "run" => Some("run"),
        "test" => Some("test"),
        "bench" => Some("bench"),
        "rustc" => Some("rustc"),
        "clippy" => Some("clippy"),
        "doc" => Some("doc"),
        _ => None,
    }
}

/// Cargo options whose following value may itself be a supported command name.
/// Tracking these prevents `--bin run` and similar arguments from being
/// mistaken for the command selected for this invocation.
fn cargo_option_takes_value(argument: &str) -> bool {
    matches!(
        argument,
        "-p" | "--package"
            | "--exclude"
            | "-j"
            | "--jobs"
            | "-F"
            | "--features"
            | "--bin"
            | "--example"
            | "--test"
            | "--bench"
            | "--profile"
            | "--manifest-path"
            | "--message-format"
            | "--target-dir"
            | "--artifact-dir"
            | "--color"
            | "--config"
    )
}

fn validate_optimize(value: &str) -> Result<String, String> {
    match value {
        "Debug" | "ReleaseSafe" | "ReleaseFast" | "ReleaseSmall" => Ok(value.into()),
        _ => Err(format!(
            "invalid -ZigOptimize '{value}'; use Debug, ReleaseSafe, ReleaseFast, or ReleaseSmall"
        )),
    }
}

fn zig_executable(zig_patch: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(path) = zig_patch {
        return validate_zig_path(path, "-zigpatch");
    }

    let home = env::var_os("Zig_home").ok_or_else(|| {
        "Zig_home is not set; set Zig_home or pass -zigpatch=<zig path>".to_owned()
    })?;
    validate_zig_path(&PathBuf::from(home), "Zig_home")
}

fn validate_zig_path(path: &Path, source: &str) -> Result<PathBuf, String> {
    if path.is_file() {
        return verify_zig(path, source);
    }
    if !path.is_dir() {
        return Err(format!("{source} path does not exist: {}", path.display()));
    }

    let names: &[&str] = if cfg!(windows) {
        &["zig.exe", "zig"]
    } else {
        &["zig"]
    };
    let executable = names
        .iter()
        .map(|name| path.join(name))
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| {
            format!(
                "could not find {} in {} directory '{}'",
                names[0],
                source,
                path.display()
            )
        })?;
    verify_zig(&executable, source)
}

fn verify_zig(executable: &Path, source: &str) -> Result<PathBuf, String> {
    let output = Command::new(executable)
        .arg("version")
        .output()
        .map_err(|err| format!("{source} does not point to an executable Zig: {err}"))?;
    if output.status.success() {
        Ok(executable.into())
    } else {
        Err(format!(
            "{source} points to '{}', but Zig version validation failed with {}",
            executable.display(),
            output.status
        ))
    }
}

fn is_android_target(target: &str) -> bool {
    matches!(
        target,
        "aarch64-linux-android"
            | "armv7-linux-androideabi"
            | "i686-linux-android"
            | "x86_64-linux-android"
    )
}

fn android_abi_for_target(target: &str) -> Option<&'static str> {
    match target {
        "aarch64-linux-android" => Some("arm64-v8a"),
        "armv7-linux-androideabi" => Some("armeabi-v7a"),
        "i686-linux-android" => Some("x86"),
        "x86_64-linux-android" => Some("x86_64"),
        _ => None,
    }
}

fn android_ndk(options: &AndroidOptions, cargo_target: &str) -> Result<AndroidNdk, String> {
    let home = env::var_os("Ndk_home").ok_or_else(|| {
        "Ndk_home is not set; set it to an Android NDK installation or Android SDK ndk directory"
            .to_owned()
    })?;
    let root = select_ndk_root(Path::new(&home), options)?;
    let version = root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("direct")
        .to_owned();
    let sysroot = root
        .join("toolchains")
        .join("llvm")
        .join("prebuilt")
        .join(ndk_host_tag())
        .join("sysroot");
    if !sysroot.is_dir() {
        return Err(format!(
            "Android NDK '{}' has no sysroot for host '{}': {}",
            root.display(),
            ndk_host_tag(),
            sysroot.display()
        ));
    }
    let bin = root
        .join("toolchains")
        .join("llvm")
        .join("prebuilt")
        .join(ndk_host_tag())
        .join("bin");
    let (compiler_prefix, default_api_level) = android_compiler_prefix(cargo_target)?;
    let api_level = options.api_level.unwrap_or(default_api_level);
    let executable_extension = if cfg!(windows) { ".cmd" } else { "" };
    let cc = bin.join(format!(
        "{compiler_prefix}{api_level}-clang{executable_extension}"
    ));
    let cxx = bin.join(format!(
        "{compiler_prefix}{api_level}-clang++{executable_extension}"
    ));
    let ar = bin.join(if cfg!(windows) {
        "llvm-ar.exe"
    } else {
        "llvm-ar"
    });
    if options.ndk_fallback {
        for (name, tool) in [
            ("C compiler", &cc),
            ("C++ compiler", &cxx),
            ("archiver", &ar),
        ] {
            if !tool.is_file() {
                return Err(format!(
                    "Android NDK '{}' does not provide the {name} needed for {cargo_target}: {}",
                    root.display(),
                    tool.display()
                ));
            }
        }
    }
    Ok(AndroidNdk {
        root,
        version,
        sysroot,
        cc,
        cxx,
        ar,
    })
}

fn android_compiler_prefix(target: &str) -> Result<(&'static str, u32), String> {
    match target {
        "aarch64-linux-android" => Ok(("aarch64-linux-android", 21)),
        "armv7-linux-androideabi" => Ok(("armv7a-linux-androideabi", 16)),
        "i686-linux-android" => Ok(("i686-linux-android", 16)),
        "x86_64-linux-android" => Ok(("x86_64-linux-android", 21)),
        _ => Err(format!("unsupported Android Rust target: {target}")),
    }
}

fn select_ndk_root(home: &Path, options: &AndroidOptions) -> Result<PathBuf, String> {
    let is_ndk_root = |path: &Path| {
        path.join("toolchains")
            .join("llvm")
            .join("prebuilt")
            .is_dir()
    };
    if is_ndk_root(home) {
        if options.ndk_version.is_some() || options.ndk_index.is_some() {
            return Err(format!(
                "Ndk_home '{}' already points to one NDK installation; remove -ndkversion/-nindx",
                home.display()
            ));
        }
        return Ok(home.into());
    }
    if !home.is_dir() {
        return Err(format!("Ndk_home path does not exist: {}", home.display()));
    }

    let candidates = fs::read_dir(home)
        .map_err(|err| format!("cannot read Ndk_home directory '{}': {err}", home.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir() && is_ndk_root(path))
        .collect::<Vec<_>>();
    let available = candidates
        .iter()
        .filter_map(|path| path.file_name()?.to_str())
        .collect::<Vec<_>>()
        .join(", ");
    select_ndk_candidate(candidates, options).ok_or_else(|| {
        if let Some(version) = &options.ndk_version {
            format!(
                "NDK version '{version}' was not found below Ndk_home '{}'; available versions: {available}",
                home.display()
            )
        } else if let Some(index) = options.ndk_index {
            format!(
                "NDK index {index} is out of range below Ndk_home '{}'; newest-first versions: {available}",
                home.display()
            )
        } else {
            format!(
                "Ndk_home '{}' is not an NDK and contains no NDK version with toolchains/llvm/prebuilt",
                home.display()
            )
        }
    })
}

fn select_ndk_candidate(mut candidates: Vec<PathBuf>, options: &AndroidOptions) -> Option<PathBuf> {
    if candidates.is_empty() {
        return None;
    }
    candidates.sort_by_key(|path| std::cmp::Reverse(ndk_version_key(path)));
    if let Some(version) = &options.ndk_version {
        return candidates.into_iter().find(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy() == version.as_str())
        });
    }
    if let Some(index) = options.ndk_index {
        return candidates.into_iter().nth(index);
    }
    candidates.into_iter().next()
}

fn ndk_version_key(path: &Path) -> Vec<u32> {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .split('.')
        .map(|piece| piece.parse().unwrap_or(0))
        .collect()
}

fn ndk_host_tag() -> &'static str {
    if cfg!(windows) {
        "windows-x86_64"
    } else if cfg!(target_os = "macos") && cfg!(target_arch = "aarch64") {
        "darwin-arm64"
    } else if cfg!(target_os = "macos") {
        "darwin-x86_64"
    } else {
        "linux-x86_64"
    }
}

struct Wrappers {
    directory: PathBuf,
    cc: PathBuf,
    cxx: PathBuf,
    linker: PathBuf,
    ar: PathBuf,
}

fn make_wrappers() -> Result<Wrappers, String> {
    let current = env::current_exe().map_err(|err| format!("cannot locate cargo-zirild: {err}"))?;
    let target_directory = match env::var_os("CARGO_TARGET_DIR") {
        Some(directory) => PathBuf::from(directory),
        None => env::current_dir()
            .map_err(|err| format!("cannot determine project directory: {err}"))?
            .join("target"),
    };
    let directory = target_directory
        .join(".cargo-zirild")
        .join(format!("{}", std::process::id()));
    fs::create_dir_all(&directory).map_err(|err| {
        format!(
            "cannot create Zig wrapper directory '{}': {err}",
            directory.display()
        )
    })?;

    let extension = if cfg!(windows) { ".exe" } else { "" };
    let wrapper = |name: &str| directory.join(format!("cargo-zirild-{name}{extension}"));
    let cc = wrapper("cc");
    let cxx = wrapper("cxx");
    let linker = wrapper("linker");
    let ar = wrapper("ar");
    let dlltool = directory.join(if cfg!(windows) {
        "dlltool.exe"
    } else {
        "dlltool"
    });
    for path in [&cc, &cxx, &linker, &ar, &dlltool] {
        fs::copy(&current, path).map_err(|err| {
            format!(
                "cannot create Zig wrapper '{}' from '{}': {err}",
                path.display(),
                current.display()
            )
        })?;
    }
    Ok(Wrappers {
        directory,
        cc,
        cxx,
        linker,
        ar,
    })
}

fn run_cargo(
    options: &BuildOptions,
    zig: Option<&Path>,
    wrappers: &Wrappers,
    android_ndk: Option<&AndroidNdk>,
) -> Result<(), String> {
    let mut command = Command::new("cargo");
    command
        .arg(&options.cargo_command)
        .arg("--target")
        .arg(&options.cargo_target)
        .args(&options.cargo_args);

    if let Some(zig) = zig {
        command.env("CARGO_ZIRILD_ZIG", zig);
    }
    command.env("CARGO_ZIRILD_ZIG_TARGET", &options.zig_target);
    command.env("CARGO_ZIRILD_OPTIMIZE", &options.optimize);
    if let Some(ndk) = android_ndk {
        command.env("CARGO_ZIRILD_ANDROID_SYSROOT", &ndk.sysroot);
        command.env(
            "CARGO_ZIRILD_ANDROID_CFLAGS",
            encode_wrapper_arguments(&options.android.cflags),
        );
        command.env(
            "CARGO_ZIRILD_ANDROID_LINK_ARGS",
            encode_wrapper_arguments(&options.android.link_args),
        );
        command.env("ANDROID_NDK_ROOT", &ndk.root);
        command.env("CMAKE_ANDROID_NDK", &ndk.root);
        if options.android.ndk_fallback {
            command.env("CARGO_ZIRILD_NDK_FALLBACK", "1");
            command.env("CARGO_ZIRILD_NDK_CC", &ndk.cc);
            command.env("CARGO_ZIRILD_NDK_CXX", &ndk.cxx);
            command.env("CARGO_ZIRILD_NDK_AR", &ndk.ar);
        }
        if let Some(abi) = &options.android.abi {
            command.env("ANDROID_ABI", abi);
        }
        if let Some(api_level) = options.android.api_level {
            command.env("ANDROID_PLATFORM", format!("android-{api_level}"));
        }
        if let Some(stl) = &options.android.stl {
            command.env("ANDROID_STL", stl);
        }
    }

    let normalized = options.cargo_target.replace('-', "_").to_ascii_uppercase();
    let is_msvc = options.cargo_target.ends_with("-msvc");
    if !is_msvc {
        command.env(
            format!("CARGO_TARGET_{normalized}_LINKER"),
            &wrappers.linker,
        );
        command.env(format!("CARGO_TARGET_{normalized}_AR"), &wrappers.ar);

        // cc crate checks both spellings depending on its version and platform.
        for target in [
            options.cargo_target.as_str(),
            &options.cargo_target.replace('-', "_"),
        ] {
            command.env(format!("CC_{target}"), &wrappers.cc);
            command.env(format!("CXX_{target}"), &wrappers.cxx);
            command.env(format!("AR_{target}"), &wrappers.ar);
        }

        let mut paths = vec![wrappers.directory.clone()];
        if let Some(current_path) = env::var_os("PATH") {
            paths.extend(env::split_paths(&current_path));
        }
        let path = env::join_paths(paths)
            .map_err(|err| format!("cannot construct PATH for Zig dlltool: {err}"))?;
        command.env("PATH", path);
    }

    if is_msvc {
        eprintln!(
            "+ cargo {} --target {} (MSVC toolchain / {})",
            options.cargo_command, options.cargo_target, options.optimize
        );
    } else if options.android.ndk_fallback {
        let ndk = android_ndk.expect("fallback is validated as Android-only");
        eprintln!(
            "cargo-zirild: WARNING: -ndkfallback enabled. This build uses Android NDK Clang/LLD directly; the final output is not linked by Zig."
        );
        eprintln!(
            "+ cargo {} --target {} (Android NDK Clang/LLD fallback {} at {} / {})",
            options.cargo_command,
            options.cargo_target,
            ndk.version,
            ndk.root.display(),
            options.optimize
        );
    } else if let Some(ndk) = android_ndk {
        eprintln!(
            "+ cargo {} --target {} (Zig {} / Android NDK {} at {} / {})",
            options.cargo_command,
            options.cargo_target,
            options.zig_target,
            ndk.version,
            ndk.root.display(),
            options.optimize
        );
    } else {
        eprintln!(
            "+ cargo {} --target {} (Zig {} / {})",
            options.cargo_command, options.cargo_target, options.zig_target, options.optimize
        );
    }
    let status = command
        .status()
        .map_err(|err| format!("failed to start cargo: {err}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "cargo {} failed with {status}",
            options.cargo_command
        ))
    }
}

fn wrapper_mode() -> Option<WrapperMode> {
    let executable = env::current_exe().ok()?;
    let stem = executable.file_stem()?.to_string_lossy();
    if stem.ends_with("-cc") {
        Some(WrapperMode::Cc)
    } else if stem.ends_with("-cxx") {
        Some(WrapperMode::Cxx)
    } else if stem.ends_with("-linker") {
        Some(WrapperMode::Linker)
    } else if stem.ends_with("-ar") {
        Some(WrapperMode::Ar)
    } else if stem == "dlltool" || stem.ends_with("-dlltool") {
        Some(WrapperMode::Dlltool)
    } else {
        None
    }
}

fn run_wrapper(mode: WrapperMode) -> Result<(), String> {
    let target = env::var_os("CARGO_ZIRILD_ZIG_TARGET")
        .ok_or_else(|| "CARGO_ZIRILD_ZIG_TARGET is missing from wrapper environment".to_owned())?;
    let optimize = env::var_os("CARGO_ZIRILD_OPTIMIZE")
        .ok_or_else(|| "CARGO_ZIRILD_OPTIMIZE is missing from wrapper environment".to_owned())?;
    let compiler_optimize = compiler_optimization_flag(&optimize)?;

    let ndk_fallback = env::var_os("CARGO_ZIRILD_NDK_FALLBACK").is_some();
    let mut command = if ndk_fallback {
        let tool = match mode {
            WrapperMode::Cc | WrapperMode::Linker => env::var_os("CARGO_ZIRILD_NDK_CC"),
            WrapperMode::Cxx => env::var_os("CARGO_ZIRILD_NDK_CXX"),
            WrapperMode::Ar => env::var_os("CARGO_ZIRILD_NDK_AR"),
            WrapperMode::Dlltool => {
                return Err(
                    "NDK fallback does not provide dlltool; Android builds must not request it"
                        .into(),
                );
            }
        }
        .ok_or_else(|| "NDK fallback tool path is missing from wrapper environment".to_owned())?;
        let mut command = Command::new(tool);
        if matches!(
            mode,
            WrapperMode::Cc | WrapperMode::Cxx | WrapperMode::Linker
        ) {
            command.arg(compiler_optimize);
        }
        command
    } else {
        let zig = env::var_os("CARGO_ZIRILD_ZIG")
            .ok_or_else(|| "CARGO_ZIRILD_ZIG is missing from wrapper environment".to_owned())?;
        let mut command = Command::new(zig);
        match mode {
            WrapperMode::Cc => {
                command
                    .arg("cc")
                    .arg("-target")
                    .arg(&target)
                    .arg(compiler_optimize);
            }
            WrapperMode::Cxx => {
                command
                    .arg("c++")
                    .arg("-target")
                    .arg(&target)
                    .arg(compiler_optimize);
            }
            WrapperMode::Linker => {
                command
                    .arg("cc")
                    .arg("-target")
                    .arg(&target)
                    .arg(compiler_optimize);
            }
            WrapperMode::Ar => {
                command.arg("ar");
            }
            WrapperMode::Dlltool => {
                command.arg("dlltool");
            }
        }
        command
    };
    if matches!(
        mode,
        WrapperMode::Cc | WrapperMode::Cxx | WrapperMode::Linker
    ) {
        let native_arguments = if mode == WrapperMode::Linker {
            wrapper_arguments("CARGO_ZIRILD_ANDROID_LINK_ARGS")
        } else {
            wrapper_arguments("CARGO_ZIRILD_ANDROID_CFLAGS")
        };
        command.args(native_arguments);
    }
    let arguments: Vec<_> = env::args_os().skip(1).collect();
    let windows_link = mode == WrapperMode::Linker
        && target
            .to_string_lossy()
            .to_ascii_lowercase()
            .contains("-windows-");
    // Rust's GNU Windows target supplies -nodefaultlibs and then expects a
    // MinGW installation. Zig ships its own Windows CRT, which is intentionally
    // disabled by that flag, so let Zig inject its bundled CRT instead.
    let response_files = if windows_link {
        filter_windows_runtime_response_files(&arguments)?
    } else {
        Vec::new()
    };
    let mut argument_index = 0;
    while argument_index < arguments.len() {
        let argument = &arguments[argument_index];
        if mode == WrapperMode::Dlltool && argument == "--temp-prefix" {
            argument_index += 2;
            continue;
        }
        if mode == WrapperMode::Linker
            && (argument == "-nodefaultlibs"
                || (windows_link
                    && (is_windows_runtime_argument(argument)
                        || is_windows_auto_image_base_argument(argument))))
        {
            argument_index += 1;
            continue;
        }
        if windows_link {
            let transformed = transform_windows_linker_argument(argument);
            if env::var_os("CARGO_ZIRILD_TRACE").is_some()
                && Path::new(&transformed)
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("def"))
            {
                match fs::read_to_string(&transformed) {
                    Ok(contents) => eprintln!(
                        "cargo-zirild trace: definition file {}:\n{}",
                        Path::new(&transformed).display(),
                        contents
                    ),
                    Err(err) => eprintln!(
                        "cargo-zirild trace: cannot read definition file {}: {err}",
                        Path::new(&transformed).display()
                    ),
                }
            }
            command.arg(transformed);
        } else {
            command.arg(argument);
        }
        argument_index += 1;
    }
    if windows_link {
        command.arg("-lunwind");
    }

    if env::var_os("CARGO_ZIRILD_TRACE").is_some() && mode != WrapperMode::Dlltool {
        eprintln!(
            "cargo-zirild trace: invoking {} in {mode:?} mode with {} arguments",
            if ndk_fallback { "NDK LLVM" } else { "Zig" },
            arguments.len(),
        );
    }
    let status = command.status();
    restore_response_files(response_files);
    let status = status.map_err(|err| format!("failed to start Zig: {err}"))?;
    if status.success() {
        Ok(())
    } else {
        let tool = if ndk_fallback { "NDK LLVM" } else { "Zig" };
        Err(format!("{tool} {mode:?} failed with {status}"))
    }
}

fn encode_wrapper_arguments(arguments: &[String]) -> String {
    arguments.join("\n")
}

fn wrapper_arguments(name: &str) -> Vec<OsString> {
    env::var(name)
        .ok()
        .map(|arguments| {
            arguments
                .split('\n')
                .filter(|argument| !argument.is_empty())
                .map(OsString::from)
                .collect()
        })
        .unwrap_or_default()
}

fn is_windows_runtime_argument(argument: &OsString) -> bool {
    matches!(
        argument.to_string_lossy().as_ref(),
        "-nodefaultlibs"
            | "-lgcc_eh"
            | "-l:libpthread.a"
            | "-lmsvcrt"
            | "-lmingwex"
            | "-lmingw32"
            | "-lgcc"
    )
}

/// Zig's bundled LLD explicitly ignores MinGW's auto-image-base switches and
/// emits a warning for them. Removing them keeps the Windows GNU build clean
/// without changing Zig's image-base behavior.
fn is_windows_auto_image_base_argument(argument: &OsString) -> bool {
    matches!(
        argument.to_string_lossy().as_ref(),
        "-Wl,--enable-auto-image-base"
            | "-Wl,--disable-auto-image-base"
            | "--enable-auto-image-base"
            | "--disable-auto-image-base"
    )
}

fn transform_windows_linker_argument(argument: &OsString) -> OsString {
    let text = argument.to_string_lossy();
    if let Some(definition) = text.strip_prefix("-Wl,")
        && definition.to_ascii_lowercase().ends_with(".def")
    {
        // Zig's compiler driver accepts a module-definition file as an input,
        // then forwards it to its Windows linker in the required form.
        return definition.into();
    }
    argument.clone()
}

/// rustc uses a response file for linker arguments on Windows. Zig provides
/// these GNU runtime libraries internally, whereas Rust's GNU target expects a
/// separate MinGW installation. Filter only those temporary response entries.
fn filter_windows_runtime_response_files(
    arguments: &[OsString],
) -> Result<Vec<(PathBuf, String)>, String> {
    let mut saved = Vec::new();
    for argument in arguments {
        let text = argument.to_string_lossy();
        let Some(path) = text.strip_prefix('@') else {
            continue;
        };
        let path = PathBuf::from(path);
        let original = fs::read_to_string(&path).map_err(|err| {
            format!(
                "cannot read linker response file '{}': {err}",
                path.display()
            )
        })?;
        let lines: Vec<_> = original.lines().collect();
        let library_directories = response_library_directories(&lines);
        patch_empty_windows_definition_files(&lines, &mut saved)?;
        let filtered = lines
            .into_iter()
            .filter_map(|line| transform_windows_response_line(line, &library_directories))
            .collect::<Vec<_>>()
            .join("\n");
        if filtered != original {
            fs::write(&path, format!("{filtered}\n")).map_err(|err| {
                format!(
                    "cannot update linker response file '{}': {err}",
                    path.display()
                )
            })?;
            saved.push((path, original));
        }
    }
    Ok(saved)
}

fn patch_empty_windows_definition_files(
    lines: &[&str],
    saved: &mut Vec<(PathBuf, String)>,
) -> Result<(), String> {
    for line in lines {
        let value = line.trim().trim_matches('"');
        let Some(definition) = value.strip_prefix("-Wl,") else {
            continue;
        };
        if !definition.to_ascii_lowercase().ends_with(".def") {
            continue;
        }
        let path = PathBuf::from(definition);
        let original = fs::read_to_string(&path).map_err(|err| {
            format!(
                "cannot read Windows definition file '{}': {err}",
                path.display()
            )
        })?;
        if let Some(patched) = windows_definition_with_fallback_export(&original) {
            fs::write(&path, patched).map_err(|err| {
                format!(
                    "cannot update Windows definition file '{}': {err}",
                    path.display()
                )
            })?;
            saved.push((path, original));
        }
    }
    Ok(())
}

fn windows_definition_with_fallback_export(contents: &str) -> Option<String> {
    let mut after_exports = false;
    for line in contents.lines() {
        let value = line.trim();
        if value.eq_ignore_ascii_case("EXPORTS") {
            after_exports = true;
            continue;
        }
        if after_exports && !value.is_empty() && !value.starts_with(';') {
            return None;
        }
    }
    after_exports.then(|| "EXPORTS\n    DllMainCRTStartup\n".into())
}

fn response_library_directories(lines: &[&str]) -> Vec<PathBuf> {
    let mut directories = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let value = lines[index].trim().trim_matches('"');
        if value == "-L" && index + 1 < lines.len() {
            directories.push(PathBuf::from(lines[index + 1].trim().trim_matches('"')));
            index += 2;
            continue;
        }
        if let Some(directory) = value.strip_prefix("-L")
            && !directory.is_empty()
        {
            directories.push(PathBuf::from(directory));
        }
        index += 1;
    }
    directories
}

fn transform_windows_response_line(line: &str, library_directories: &[PathBuf]) -> Option<String> {
    let trimmed = line.trim();
    let quoted = trimmed.starts_with('"') && trimmed.ends_with('"');
    let value = trimmed.trim_matches('"');
    let argument = OsString::from(value);
    if is_windows_runtime_argument(&argument) || is_windows_auto_image_base_argument(&argument) {
        return None;
    }

    if let Some(name) = value.strip_prefix("-l")
        && let Some(library) = find_windows_import_library(name, library_directories)
    {
        return Some(format!("\"{}\"", library.display()));
    }

    let transformed = transform_windows_linker_argument(&OsString::from(value));
    if transformed == value {
        Some(line.into())
    } else if quoted {
        Some(format!("\"{}\"", transformed.to_string_lossy()))
    } else {
        Some(transformed.to_string_lossy().into_owned())
    }
}

fn find_windows_import_library(name: &str, directories: &[PathBuf]) -> Option<PathBuf> {
    let candidates = [
        format!("lib{name}.a"),
        format!("{name}.lib"),
        format!("lib{name}.dll.a"),
    ];
    for directory in directories {
        for candidate in &candidates {
            let path = directory.join(candidate);
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}

fn restore_response_files(response_files: Vec<(PathBuf, String)>) {
    for (path, original) in response_files {
        let _ = fs::write(path, original);
    }
}

/// Zig's C/C++ frontend accepts Clang optimization flags, not Zig Build's
/// -OReleaseSafe spelling. Rust optimization remains controlled by Cargo.
fn compiler_optimization_flag(optimize: &OsString) -> Result<&'static str, String> {
    match optimize.to_string_lossy().as_ref() {
        "Debug" => Ok("-O0"),
        "ReleaseSafe" => Ok("-O2"),
        "ReleaseFast" => Ok("-O3"),
        "ReleaseSmall" => Ok("-Oz"),
        invalid => Err(format!("invalid CARGO_ZIRILD_OPTIMIZE '{invalid}'")),
    }
}

/// Convert common Zig triples to the Rust triples required by cargo --target.
/// Fully qualified Rust triples are preserved.
fn cargo_target_for(zig_target: &str) -> String {
    let pieces: Vec<_> = zig_target.split('-').collect();
    match pieces.as_slice() {
        ["arm", "linux", "android"] | ["armv7", "linux", "android"] => {
            "armv7-linux-androideabi".into()
        }
        [architecture, "linux", "android"] => format!("{architecture}-linux-android"),
        [architecture, "linux", abi] => format!("{architecture}-unknown-linux-{abi}"),
        [architecture, "pc", "windows"] => format!("{architecture}-pc-windows-msvc"),
        [architecture, "windows", abi] => format!("{architecture}-pc-windows-{abi}"),
        [architecture, "macos", _] => format!("{architecture}-apple-darwin"),
        _ => zig_target.into(),
    }
}

/// Accept a Rust target triple while passing Zig's shorter platform spelling
/// to its C/C++ and linker frontends.
fn zig_target_for(target: &str) -> String {
    let pieces: Vec<_> = target.split('-').collect();
    match pieces.as_slice() {
        [architecture, "unknown", "linux", abi] => format!("{architecture}-linux-{abi}"),
        [architecture, "pc", "windows"] => format!("{architecture}-windows-msvc"),
        [architecture, "pc", "windows", abi] => format!("{architecture}-windows-{abi}"),
        [architecture, "apple", "darwin"] => format!("{architecture}-macos-none"),
        _ => target.into(),
    }
}

fn warn_about_android_target(zig_target: &str, ndk: Option<&AndroidNdk>, ndk_fallback: bool) {
    if let Some(ndk) = ndk {
        if ndk_fallback {
            eprintln!(
                "cargo-zirild: Android NDK {} selected at '{}'; -ndkfallback will use its Clang/LLD tools directly.",
                ndk.version,
                ndk.root.display()
            );
        } else {
            eprintln!(
                "cargo-zirild: Android NDK {} selected at '{}'; its root and sysroot are exported to build scripts while Zig remains the compiler and final linker.",
                ndk.version,
                ndk.root.display()
            );
        }
    }
    if zig_target.contains("-android") && !ndk_fallback {
        eprintln!(
            "cargo-zirild: note: this Zig distribution cannot currently link Android libc, including an NDK sysroot. \
             Zirild will not fall back to NDK clang because that would violate its single Zig linker contract. \
             For a musl-based Android-compatible native build, try -target={} instead; \
             this is a musl Linux binary and not an Android-NDK-linked binary.",
            android_musl_target(zig_target)
        );
    }
}

fn android_musl_target(zig_target: &str) -> String {
    zig_target
        .split_once("-android")
        .map(|(prefix, _)| format!("{prefix}-musl"))
        .unwrap_or_else(|| zig_target.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_optimization_modes() {
        let dev = parse_args(vec!["-target=x86_64-linux-musl".into()]).unwrap();
        let release =
            parse_args(vec!["-target=x86_64-linux-musl".into(), "--release".into()]).unwrap();
        let small = parse_args(vec!["-target=x86_64-linux-musl".into(), "-Z".into()]).unwrap();
        assert_eq!(dev.optimize, "ReleaseSafe");
        assert_eq!(release.optimize, "ReleaseFast");
        assert_eq!(small.optimize, "ReleaseSmall");
    }

    #[test]
    fn explicit_optimization_wins() {
        let options = parse_args(vec![
            "-target=x86_64-linux-musl".into(),
            "--release".into(),
            "-ZigOptimize=Debug".into(),
        ])
        .unwrap();
        assert_eq!(options.optimize, "Debug");
    }

    #[test]
    fn maps_common_zig_triples_to_cargo_triples() {
        assert_eq!(
            cargo_target_for("aarch64-linux-musl"),
            "aarch64-unknown-linux-musl"
        );
        assert_eq!(
            cargo_target_for("x86_64-windows-gnu"),
            "x86_64-pc-windows-gnu"
        );
        assert_eq!(
            cargo_target_for("x86_64-linux-android"),
            "x86_64-linux-android"
        );
        assert_eq!(
            cargo_target_for("x86_64-pc-windows"),
            "x86_64-pc-windows-msvc"
        );
    }

    #[test]
    fn maps_zig_policies_to_c_compiler_flags() {
        assert_eq!(compiler_optimization_flag(&"Debug".into()).unwrap(), "-O0");
        assert_eq!(
            compiler_optimization_flag(&"ReleaseSafe".into()).unwrap(),
            "-O2"
        );
        assert_eq!(
            compiler_optimization_flag(&"ReleaseFast".into()).unwrap(),
            "-O3"
        );
        assert_eq!(
            compiler_optimization_flag(&"ReleaseSmall".into()).unwrap(),
            "-Oz"
        );
    }

    #[test]
    fn accepts_rust_linux_triples_for_zig() {
        let options = parse_args(vec!["-target=x86_64-unknown-linux-gnu".into()]).unwrap();
        assert_eq!(options.cargo_target, "x86_64-unknown-linux-gnu");
        assert_eq!(options.zig_target, "x86_64-linux-gnu");
    }

    #[test]
    fn expands_short_windows_target_to_msvc() {
        let options = parse_args(vec!["-target=x86_64-pc-windows".into()]).unwrap();
        assert_eq!(options.cargo_target, "x86_64-pc-windows-msvc");
        assert_eq!(options.zig_target, "x86_64-windows-msvc");
    }

    #[test]
    fn zigpatch_overrides_are_parsed() {
        let options = parse_args(vec![
            "-target=x86_64-linux-musl".into(),
            "-zigpatch=C:\\tools\\zig\\zig.exe".into(),
        ])
        .unwrap();
        assert_eq!(
            options.zig_patch,
            Some(PathBuf::from("C:\\tools\\zig\\zig.exe"))
        );
    }

    #[test]
    fn cargo_subcommand_name_is_not_forwarded_to_build() {
        let options =
            parse_args(vec!["-target=x86_64-linux-musl".into(), "zirild".into()]).unwrap();
        assert!(options.cargo_args.is_empty());
        assert_eq!(options.cargo_command, "build");
        assert_eq!(options.zig_target, "x86_64-linux-musl");
    }

    #[test]
    fn selects_cargo_run_after_zirild_options() {
        let options = parse_args(vec![
            "-target=x86_64-pc-windows-msvc".into(),
            "run".into(),
            "--release".into(),
        ])
        .unwrap();
        assert_eq!(options.cargo_command, "run");
        assert_eq!(options.cargo_args, vec![OsString::from("--release")]);
        assert_eq!(options.optimize, "ReleaseFast");
    }

    #[test]
    fn preserves_separator_and_run_arguments() {
        let options = parse_args(vec![
            "-target=x86_64-pc-windows-msvc".into(),
            "run".into(),
            "--".into(),
            "--help".into(),
            "run".into(),
        ])
        .unwrap();
        assert_eq!(options.cargo_command, "run");
        assert_eq!(
            options.cargo_args,
            vec![
                OsString::from("--"),
                OsString::from("--help"),
                OsString::from("run")
            ]
        );
    }

    #[test]
    fn does_not_treat_cargo_option_value_as_command() {
        let options = parse_args(vec![
            "-target=x86_64-pc-windows-msvc".into(),
            "--bin".into(),
            "run".into(),
        ])
        .unwrap();
        assert_eq!(options.cargo_command, "build");
        assert_eq!(
            options.cargo_args,
            vec![OsString::from("--bin"), OsString::from("run")]
        );
    }

    #[test]
    fn converts_windows_definition_file_for_zig_linker() {
        let argument = OsString::from("-Wl,D:\\build\\exports.def");
        assert_eq!(
            transform_windows_linker_argument(&argument),
            OsString::from("D:\\build\\exports.def")
        );
    }

    #[test]
    fn converts_definition_files_inside_linker_response_files() {
        assert_eq!(
            transform_windows_response_line("\"-Wl,D:\\build\\exports.def\"", &[]),
            Some("\"D:\\build\\exports.def\"".into())
        );
        assert_eq!(
            transform_windows_response_line("\"-nodefaultlibs\"", &[]),
            None
        );
        assert_eq!(
            transform_windows_response_line("\"-Wl,--disable-auto-image-base\"", &[]),
            None
        );
    }

    #[test]
    fn adds_fallback_export_only_to_empty_definition_files() {
        assert_eq!(
            windows_definition_with_fallback_export("EXPORTS\n"),
            Some("EXPORTS\n    DllMainCRTStartup\n".into())
        );
        assert_eq!(
            windows_definition_with_fallback_export("EXPORTS\n    real_export\n"),
            None
        );
    }

    #[test]
    fn orders_ndk_version_directories_numerically() {
        assert!(
            ndk_version_key(Path::new("27.2.12479018"))
                < ndk_version_key(Path::new("30.0.14904198"))
        );
    }

    #[test]
    fn selects_ndk_versions_newest_first_or_by_exact_name() {
        let candidates = vec![
            PathBuf::from("27.2.12479018"),
            PathBuf::from("30.0.14904198"),
            PathBuf::from("26.3.11579264"),
        ];
        assert_eq!(
            select_ndk_candidate(candidates.clone(), &AndroidOptions::default()),
            Some(PathBuf::from("30.0.14904198"))
        );
        let indexed = AndroidOptions {
            ndk_index: Some(1),
            ..AndroidOptions::default()
        };
        assert_eq!(
            select_ndk_candidate(candidates.clone(), &indexed),
            Some(PathBuf::from("27.2.12479018"))
        );
        let exact = AndroidOptions {
            ndk_version: Some("26.3.11579264".into()),
            ..AndroidOptions::default()
        };
        assert_eq!(
            select_ndk_candidate(candidates, &exact),
            Some(PathBuf::from("26.3.11579264"))
        );
    }

    #[test]
    fn parses_android_ndk_and_toolchain_options_without_forwarding_to_cargo() {
        let options = parse_args(vec![
            "-target=x86_64-linux-android".into(),
            "-nindx=1".into(),
            "-android-api=24".into(),
            "-DANDROID_ABI=x86_64".into(),
            "-DANDROID_STL=c++_shared".into(),
            "-android-cflag=-fPIC".into(),
            "-android-link-arg=-Wl,--build-id".into(),
            "-ndkfallback".into(),
        ])
        .unwrap();
        assert_eq!(options.zig_target, "x86_64-linux-android.24");
        assert_eq!(options.android.ndk_index, Some(1));
        assert_eq!(options.android.api_level, Some(24));
        assert_eq!(options.android.abi.as_deref(), Some("x86_64"));
        assert_eq!(options.android.stl.as_deref(), Some("c++_shared"));
        assert_eq!(options.android.cflags, vec!["-fPIC"]);
        assert_eq!(options.android.link_args, vec!["-Wl,--build-id"]);
        assert!(options.android.ndk_fallback);
        assert!(options.cargo_args.is_empty());
    }

    #[test]
    fn rejects_conflicting_ndk_selectors_and_android_abi() {
        assert!(
            parse_args(vec![
                "-target=x86_64-linux-android".into(),
                "-ndkversion=30.0.14904198".into(),
                "-nindx=0".into(),
            ])
            .is_err()
        );
        assert!(
            parse_args(vec![
                "-target=x86_64-linux-android".into(),
                "-android-abi=arm64-v8a".into(),
            ])
            .is_err()
        );
    }

    #[test]
    fn removes_android_api_level_from_musl_compatibility_suggestion() {
        assert_eq!(
            android_musl_target("x86_64-linux-android.24"),
            "x86_64-linux-musl"
        );
    }
}
