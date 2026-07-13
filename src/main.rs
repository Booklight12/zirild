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
    Dlltool,
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
    let mut args: Vec<_> = args.into_iter().collect();
    // Cargo passes the external subcommand name as argv[1], so
    // cargo zirild ... arrives as cargo-zirild zirild ....
    if args.first().is_some_and(|argument| argument == "zirild") {
        args.remove(0);
    }
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
        // Cargo may inject the external subcommand name before or after user
        // options, depending on the invoking Cargo version and platform.
        if argument == "zirild" {
            continue;
        } else if passthrough {
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
            "+ cargo build --target {} (MSVC toolchain / {})",
            options.cargo_target, options.optimize
        );
    } else {
        eprintln!(
            "+ cargo build --target {} (Zig {} / {})",
            options.cargo_target, options.zig_target, options.optimize
        );
    }
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
    } else if stem == "dlltool" || stem.ends_with("-dlltool") {
        Some(WrapperMode::Dlltool)
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
        WrapperMode::Dlltool => {
            command.arg("dlltool");
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
    let mut argument_index = 0;
    while argument_index < arguments.len() {
        let argument = &arguments[argument_index];
        if mode == WrapperMode::Dlltool && argument == "--temp-prefix" {
            argument_index += 2;
            continue;
        }
        if mode == WrapperMode::Linker
            && (argument == "-nodefaultlibs"
                || (windows_link && is_windows_runtime_argument(argument)))
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

    if env::var_os("CARGO_ZIRILD_TRACE").is_some() {
        if mode != WrapperMode::Dlltool {
            eprintln!(
                "cargo-zirild trace: invoking Zig in {mode:?} mode with {} arguments",
                arguments.len()
            );
        }
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
    if is_windows_runtime_argument(&OsString::from(value)) {
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
        assert_eq!(options.zig_target, "x86_64-linux-musl");
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
}
