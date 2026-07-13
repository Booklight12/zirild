//! cargo-zirild: build a Cargo project through Zig's C/C++ and linker tools.

use std::{
    env,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

const USAGE: &str = r#"cargo-zirild - build a Rust Cargo project with the Zig toolchain

Usage:
  cargo zirild -target=<zig-target> [options] [cargo build options]

Required:
  -target=<zig-target>        Zig target, e.g. x86_64-linux-musl

Zig optimisation:
  (default/dev)               ReleaseSafe
  --release                   ReleaseFast
  -Z                          ReleaseSmall
  -ZigOptimize=<mode>         Explicit: Debug, ReleaseSafe, ReleaseFast, ReleaseSmall

Other options:
  -zigpatch=<path>            Use this Zig executable/directory before Zig_home
  -h, --help                  Show this help

All remaining arguments are passed to cargo build, for example --features,
--package, --bin, and --locked. Do not pass Cargo's --target: -target is the
single source of truth and is converted to Cargo's corresponding Rust triple.

Zig location:
  Set Zig_home to the Zig installation directory or the full path to zig/zig.exe.
"#;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WrapperMode {
    Cc,
    Cxx,
    Linker,
    Ar,
}

#[derive(Debug)]
struct BuildOptions {
    zig_target: String,
    cargo_target: String,
    optimize: String,
    zig_patch: Option<PathBuf>,
    cargo_args: Vec<OsString>,
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
    let args: Vec<_> = args.into_iter().collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print!("{USAGE}");
        return Ok(());
    }

    let options = parse_args(args)?;
    let zig = zig_executable(options.zig_patch.as_deref())?;
    warn_about_android_target(&options.zig_target);
    let wrappers = make_wrappers()?;
    let result = run_cargo(&options, &zig, &wrappers);
    let _ = fs::remove_dir_all(&wrappers.directory);
    result
}

fn parse_args(args: Vec<OsString>) -> Result<BuildOptions, String> {
    let mut target = None;
    let mut explicit_optimize = None;
    let mut zig_patch = None;
    let mut release = false;
    let mut small = false;
    let mut cargo_args = Vec::new();
    let mut passthrough = false;

    for argument in args {
        let text = argument.to_string_lossy();
        if passthrough {
            cargo_args.push(argument);
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
        } else if argument == "--release" {
            release = true;
            cargo_args.push(argument);
        } else if argument == "-Z" {
            small = true;
        } else if argument == "--" {
            passthrough = true;
        } else if argument == "--target" || argument == "-target" {
            return Err("use -target=<zig-target>, not a separate Cargo --target".into());
        } else {
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
    let zig_target = zig_target_for(&target);

    Ok(BuildOptions {
        zig_target,
        cargo_target,
        optimize,
        zig_patch,
        cargo_args,
    })
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

struct Wrappers {
    directory: PathBuf,
    cc: PathBuf,
    cxx: PathBuf,
    linker: PathBuf,
    ar: PathBuf,
}

fn make_wrappers() -> Result<Wrappers, String> {
    let current = env::current_exe().map_err(|err| format!("cannot locate cargo-zirild: {err}"))?;
    let directory = env::current_dir()
        .map_err(|err| format!("cannot determine project directory: {err}"))?
        .join("target")
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
    for path in [&cc, &cxx, &linker, &ar] {
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

fn run_cargo(options: &BuildOptions, zig: &Path, wrappers: &Wrappers) -> Result<(), String> {
    let mut command = Command::new("cargo");
    command
        .arg("build")
        .arg("--target")
        .arg(&options.cargo_target)
        .args(&options.cargo_args);

    command.env("CARGO_ZIRILD_ZIG", zig);
    command.env("CARGO_ZIRILD_ZIG_TARGET", &options.zig_target);
    command.env("CARGO_ZIRILD_OPTIMIZE", &options.optimize);

    let normalized = options.cargo_target.replace('-', "_").to_ascii_uppercase();
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

    eprintln!(
        "+ cargo build --target {} (Zig {} / {})",
        options.cargo_target, options.zig_target, options.optimize
    );
    let status = command
        .status()
        .map_err(|err| format!("failed to start cargo: {err}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("cargo build failed with {status}"))
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
    } else {
        None
    }
}

fn run_wrapper(mode: WrapperMode) -> Result<(), String> {
    let zig = env::var_os("CARGO_ZIRILD_ZIG")
        .ok_or_else(|| "CARGO_ZIRILD_ZIG is missing from wrapper environment".to_owned())?;
    let target = env::var_os("CARGO_ZIRILD_ZIG_TARGET")
        .ok_or_else(|| "CARGO_ZIRILD_ZIG_TARGET is missing from wrapper environment".to_owned())?;
    let optimize = env::var_os("CARGO_ZIRILD_OPTIMIZE")
        .ok_or_else(|| "CARGO_ZIRILD_OPTIMIZE is missing from wrapper environment".to_owned())?;
    let compiler_optimize = compiler_optimization_flag(&optimize)?;

    let mut command = Command::new(zig);
    match mode {
        WrapperMode::Cc => {
            command
                .arg("cc")
                .arg("-target")
                .arg(&target)
                .arg(&compiler_optimize);
        }
        WrapperMode::Cxx => {
            command
                .arg("c++")
                .arg("-target")
                .arg(&target)
                .arg(&compiler_optimize);
        }
        WrapperMode::Linker => {
            command
                .arg("cc")
                .arg("-target")
                .arg(&target)
                .arg(&compiler_optimize);
        }
        WrapperMode::Ar => {
            command.arg("ar");
        }
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
    command.args(arguments.iter().filter(|argument| {
        !(mode == WrapperMode::Linker
            && (*argument == "-nodefaultlibs"
                || (windows_link && is_windows_runtime_argument(argument))))
    }));
    if windows_link {
        command.arg("-lunwind");
    }

    let status = command.status();
    restore_response_files(response_files);
    let status = status.map_err(|err| format!("failed to start Zig: {err}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("Zig {mode:?} failed with {status}"))
    }
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
        let filtered = original
            .lines()
            .filter(|line| {
                let value = line.trim().trim_matches('"');
                !is_windows_runtime_argument(&OsString::from(value))
            })
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
        [architecture, "linux", "android"] => format!("{architecture}-linux-android"),
        [architecture, "linux", abi] => format!("{architecture}-unknown-linux-{abi}"),
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
        [architecture, "pc", "windows", abi] => format!("{architecture}-windows-{abi}"),
        [architecture, "apple", "darwin"] => format!("{architecture}-macos-none"),
        _ => target.into(),
    }
}

fn warn_about_android_target(zig_target: &str) {
    if zig_target.contains("-android") {
        eprintln!(
            "cargo-zirild: note: this Zig distribution has no bundled Android libc. \
             For a musl-based Android-compatible native build, try -target={} instead; \
             this is a musl Linux binary and not an Android-NDK-linked binary.",
            zig_target.replacen("-android", "-musl", 1)
        );
    }
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
}
