//! A small `cargo` subcommand that drives Zig's C and C++ cross compiler.

use std::{
    env,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

const USAGE: &str = r#"cargo-zirild - build C/C++ sources with Zig

Usage:
  cargo zirild --target <zig-target> --output <file> [options] <source>...

Required:
  --target <zig-target>  Zig target triple, e.g. x86_64-windows-gnu
  --output <file>        Final executable, library, or other linker output

Options:
  -I, --include <dir>    Add an include directory (repeatable)
  -D, --define <name>    Add a preprocessor definition (repeatable)
  -L, --library-dir <d>  Add a library search directory (repeatable)
  -l, --library <name>   Link a library (repeatable)
  --cflag <flag>         Pass a flag to C compilation (repeatable)
  --cxxflag <flag>       Pass a flag to C++ compilation (repeatable)
  --link-flag <flag>     Pass a flag to the final link (repeatable)
  --keep-objects         Keep intermediate object files
  -h, --help             Show this help

Zig location:
  Set Zig_home to either the Zig installation directory or the full path to
  the zig executable.  The tool looks for zig.exe on Windows and zig elsewhere.

Examples:
  cargo zirild --target x86_64-windows-gnu --output build/app.exe src/main.c
  cargo zirild --target aarch64-linux-musl --output build/app \
    -I include -l pthread src/main.cpp src/support.c
"#;

#[derive(Debug, Default)]
struct BuildOptions {
    target: Option<OsString>,
    output: Option<PathBuf>,
    includes: Vec<OsString>,
    defines: Vec<OsString>,
    library_dirs: Vec<OsString>,
    libraries: Vec<OsString>,
    cflags: Vec<OsString>,
    cxxflags: Vec<OsString>,
    link_flags: Vec<OsString>,
    sources: Vec<PathBuf>,
    keep_objects: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Language {
    C,
    Cpp,
}

fn main() -> ExitCode {
    match run(env::args_os().skip(1)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("cargo-zirild: {message}");
            eprintln!("Run `cargo zirild --help` for usage.");
            ExitCode::FAILURE
        }
    }
}

fn run(args: impl IntoIterator<Item = OsString>) -> Result<(), String> {
    let args: Vec<_> = args.into_iter().collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print!("{USAGE}");
        return Ok(());
    }

    let options = parse_args(args)?;
    let zig = zig_executable()?;
    build(&zig, &options)
}

fn parse_args(args: Vec<OsString>) -> Result<BuildOptions, String> {
    let mut options = BuildOptions::default();
    let mut arguments = args.into_iter();

    while let Some(arg) = arguments.next() {
        let value = |option: &str, arguments: &mut std::vec::IntoIter<OsString>| {
            arguments
                .next()
                .ok_or_else(|| format!("{option} requires a value"))
        };

        match arg.to_string_lossy().as_ref() {
            "--target" => options.target = Some(value("--target", &mut arguments)?),
            "--output" | "-o" => {
                options.output = Some(PathBuf::from(value("--output", &mut arguments)?))
            }
            "--include" | "-I" => options.includes.push(value("--include", &mut arguments)?),
            "--define" | "-D" => options.defines.push(value("--define", &mut arguments)?),
            "--library-dir" | "-L" => options
                .library_dirs
                .push(value("--library-dir", &mut arguments)?),
            "--library" | "-l" => options.libraries.push(value("--library", &mut arguments)?),
            "--cflag" => options.cflags.push(value("--cflag", &mut arguments)?),
            "--cxxflag" => options.cxxflags.push(value("--cxxflag", &mut arguments)?),
            "--link-flag" => options
                .link_flags
                .push(value("--link-flag", &mut arguments)?),
            "--keep-objects" => options.keep_objects = true,
            "--" => {
                options.sources.extend(arguments.map(PathBuf::from));
                break;
            }
            _ if arg.to_string_lossy().starts_with('-') => {
                return Err(format!("unknown option `{}`", arg.to_string_lossy()));
            }
            _ => options.sources.push(PathBuf::from(arg)),
        }
    }

    if options.target.is_none() {
        return Err("missing required --target <zig-target>".into());
    }
    if options.output.is_none() {
        return Err("missing required --output <file>".into());
    }
    if options.sources.is_empty() {
        return Err("at least one .c, .cc, .cpp, .cxx, or .c++ source is required".into());
    }
    for source in &options.sources {
        source_language(source)?;
    }

    Ok(options)
}

fn zig_executable() -> Result<PathBuf, String> {
    let home = env::var_os("Zig_home").ok_or_else(|| {
        "Zig_home is not set; point it at Zig's directory or executable".to_owned()
    })?;
    let path = PathBuf::from(home);

    if path.is_file() {
        return Ok(path);
    }
    if !path.is_dir() {
        return Err(format!("Zig_home path does not exist: {}", path.display()));
    }

    let names: &[&str] = if cfg!(windows) {
        &["zig.exe", "zig"]
    } else {
        &["zig"]
    };
    for name in names {
        let executable = path.join(name);
        if executable.is_file() {
            return Ok(executable);
        }
    }
    Err(format!(
        "could not find {} in Zig_home directory `{}`",
        names[0],
        path.display()
    ))
}

fn build(zig: &Path, options: &BuildOptions) -> Result<(), String> {
    let output = options.output.as_ref().expect("validated by parse_args");
    let target = options.target.as_ref().expect("validated by parse_args");
    let object_dir = object_directory(output);
    fs::create_dir_all(&object_dir).map_err(|err| {
        format!(
            "cannot create object directory `{}`: {err}",
            object_dir.display()
        )
    })?;

    let mut objects = Vec::with_capacity(options.sources.len());
    let mut has_cpp = false;
    let result = (|| {
        for (index, source) in options.sources.iter().enumerate() {
            let language = source_language(source)?;
            has_cpp |= language == Language::Cpp;
            let object = object_dir.join(format!("{index}-{}.o", source_file_stem(source)));
            compile(zig, target, source, &object, language, options)?;
            objects.push(object);
        }
        link(zig, target, &objects, output, has_cpp, options)
    })();

    if !options.keep_objects {
        cleanup(&objects, &object_dir);
    }
    result
}

fn compile(
    zig: &Path,
    target: &OsString,
    source: &Path,
    object: &Path,
    language: Language,
    options: &BuildOptions,
) -> Result<(), String> {
    let mut command = Command::new(zig);
    command.arg(match language {
        Language::C => "cc",
        Language::Cpp => "c++",
    });
    command
        .arg("-target")
        .arg(target)
        .arg("-c")
        .arg(source)
        .arg("-o")
        .arg(object);
    add_common_compile_options(&mut command, options);
    match language {
        Language::C => command.args(&options.cflags),
        Language::Cpp => command.args(&options.cxxflags),
    };
    execute(command, "compile")
}

fn link(
    zig: &Path,
    target: &OsString,
    objects: &[PathBuf],
    output: &Path,
    has_cpp: bool,
    options: &BuildOptions,
) -> Result<(), String> {
    let mut command = Command::new(zig);
    command.arg(if has_cpp { "c++" } else { "cc" });
    command
        .arg("-target")
        .arg(target)
        .args(objects)
        .arg("-o")
        .arg(output);
    for directory in &options.library_dirs {
        command.arg("-L").arg(directory);
    }
    for library in &options.libraries {
        command.arg("-l").arg(library);
    }
    command.args(&options.link_flags);
    execute(command, "link")
}

fn add_common_compile_options(command: &mut Command, options: &BuildOptions) {
    for include in &options.includes {
        command.arg("-I").arg(include);
    }
    for define in &options.defines {
        command.arg("-D").arg(define);
    }
}

fn execute(mut command: Command, stage: &str) -> Result<(), String> {
    eprintln!("+ {command:?}");
    let status = command
        .status()
        .map_err(|err| format!("failed to start Zig during {stage}: {err}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("Zig {stage} failed with {status}"))
    }
}

fn source_language(source: &Path) -> Result<Language, String> {
    let extension = source
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase());
    match extension.as_deref() {
        Some("c") => Ok(Language::C),
        Some("cc" | "cpp" | "cxx" | "c++") => Ok(Language::Cpp),
        _ => Err(format!("unsupported source `{}`", source.display())),
    }
}

fn source_file_stem(source: &Path) -> String {
    source
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("source")
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' || character == '-' {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn object_directory(output: &Path) -> PathBuf {
    let parent = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let name = output
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("output");
    parent.join(format!(".{name}.zirild-objects"))
}

fn cleanup(objects: &[PathBuf], object_dir: &Path) {
    for object in objects {
        let _ = fs::remove_file(object);
    }
    if fs::read_dir(object_dir)
        .ok()
        .and_then(|mut entries| entries.next())
        .is_none()
    {
        let _ = fs::remove_dir(object_dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_c_and_cpp_build() {
        let options = parse_args(vec![
            "--target".into(),
            "aarch64-linux-musl".into(),
            "--output".into(),
            "bin/app".into(),
            "-I".into(),
            "include".into(),
            "main.c".into(),
            "support.cpp".into(),
        ])
        .unwrap();
        assert_eq!(options.target, Some("aarch64-linux-musl".into()));
        assert_eq!(options.sources.len(), 2);
        assert_eq!(source_language(&options.sources[1]).unwrap(), Language::Cpp);
    }

    #[test]
    fn rejects_missing_target() {
        let error = parse_args(vec!["--output".into(), "app".into(), "main.c".into()]).unwrap_err();
        assert!(error.contains("--target"));
    }

    #[test]
    fn object_directory_is_next_to_output() {
        assert_eq!(
            object_directory(Path::new("build/app.exe")),
            PathBuf::from("build/.app.exe.zirild-objects")
        );
    }
}
