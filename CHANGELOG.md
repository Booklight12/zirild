# Changelog

## Unreleased

- Stop reserving Cargo's `-Z` option; add `--zig-release-small` and the canonical
  `--zig-opt=<mode>` replacement for Zirild optimization selection.
- Stop guessing Cargo commands after Cargo argument parsing begins, preventing
  future option values such as `run` from being reinterpreted as commands.
- Rewrite Windows GNU response and empty module-definition files through
  private copies instead of modifying rustc-generated inputs in place.
- Add `--windows-runtime=auto|zig|preserve`, a preservation alias, and warnings
  for detected custom Windows startup/runtime arguments.
- Respect Cargo build-helper C/C++ optimization flags by default; only explicit
  Zirild optimization selection removes conflicting native `-O*` flags and
  appends the selected policy last.
- Add standard Zig and Android NDK environment lookup, `PATH` fallback,
  `--zig-path`, `--ndk-path`, and a public `--trace` option while retaining the
  legacy mixed-case environment variables and CLI spellings.
- Report NDK LLVM, rather than Zig, when an Android fallback tool cannot start.
- Ignore duplicate `--target` arguments injected by native build helpers such
  as `cc-rs`, preserving Zirild's Zig-compatible target mapping for C and C++.
- Disable Zig's implicit C/C++ undefined-behavior sanitizer instrumentation by
  default so mixed Rust/native links do not require an unrequested UBSan
  runtime. Explicit sanitizer flags can still override this default.
- Validate mixed Rust, C, and C++ builds for Linux GNU, Windows GNU, and Windows
  MSVC; execute the Linux ELF in Ubuntu on WSL2 and both PE executables on the
  Windows host.

## 0.1.5 - 2026-07-14

- Add Android NDK discovery from `Ndk_home`, selecting the newest installed
  version by default, an exact `-ndkversion=<version>` selector, and the
  newest-first `-nindx=<index>` selector.
- Parse Android API, ABI, STL, native compiler/linker flags, and common
  CMake-style `-DANDROID_*` configuration values without forwarding them to
  Cargo as invalid arguments.
- Add explicit `-ndkfallback` support for Android NDK Clang/LLD and LLVM ar.
  This emits a prominent warning because the final file is no longer linked by
  Zig.
- Validate the five locally installed Rust targets against a temporary external
  fixture, including NDK-linked Android ELF outputs.
- Filter the Windows GNU auto-image-base switches that Zig LLD ignores, removing
  its non-actionable linker warning.

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
