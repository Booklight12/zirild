# Changelog

## 0.1.4 - 2026-07-14

- Add Cargo command selection for build, check, run, test, bench, rustc,
  clippy, and doc, while keeping build as the default.
- Support `cargo zirild -target=<target> run` and preserve executable arguments
  following `--`.
- Avoid confusing Cargo option values such as `--bin run` with the selected
  Cargo command.
- Verify native execution for Windows MSVC and Windows GNU, full linking for
  Linux GNU, and target checking for the locally installed Android targets.

## 0.1.3 - 2026-07-14

- Fix Cargo's injected zirild subcommand argument from being forwarded to the
  internal cargo build command.
- Support x86_64-pc-windows as shorthand for the MSVC Rust target, using the
  system MSVC compiler, linker, and librarian.
- Provide zig dlltool for Windows GNU raw-dylib import libraries and normalize
  its LLVM-compatible arguments.
- Translate Windows GNU response files for Zig, including module-definition
  files, third-party import archives, CRT libraries, and empty DLL exports.
- Verify x86_64-unknown-linux-gnu builds through Zig and document the sysroot
  requirement for Linux desktop libraries outside libc.

## 0.1.1 - 2026-07-14

- Declares the BSD 2-Clause license with copyright held by SorMaze.
- Adds crates.io publication metadata for the project repository.

## 0.1.0 - 2026-07-14

Initial release.

- Provides the cargo zirild subcommand for Cargo project builds through Zig.
- Configures temporary Zig wrappers for Rust linking plus native C, C++, and
  archive build steps without requiring a Cargo configuration file.
- Supports Zig target input and common Rust target triples, including
  x86_64-unknown-linux-gnu.
- Provides Zig optimization policies through default/dev, --release, -Z,
  and -ZigOptimize=<mode>.
- Resolves and validates Zig from Zig_home, with higher-priority
  -zigpatch=<path> overrides.
- Documents Android libc limitations and musl-target guidance.
