# cargo-zirild

[GitHub repository](https://github.com/Booklight12/zirild)

`cargo-zirild` is a Cargo subcommand for building, checking, testing, and
running Rust projects for a selected target. It creates temporary tool wrappers
for Rust linking and native C/C++ dependencies, so projects do not need a
custom `.cargo/config.toml` or manually configured `CC`, `CXX`, linker, `AR`,
or `dlltool` variables. The executable has no third-party Rust dependencies.

The long-term goal is one cross-platform final-link path driven by Zig. Rust
source is still compiled by `rustc`; Zirild controls the native compiler,
archiver, and linker selected around Cargo.

## Toolchain policy

| Target or mode | C/C++ and archives | Final linker | Current status |
| --- | --- | --- | --- |
| GNU-family targets | `zig cc`, `zig c++`, `zig ar` | Zig | primary path |
| Windows GNU raw-dylib | Zig tools and `zig dlltool` | Zig | supported |
| Windows MSVC | system MSVC tools | system MSVC linker | compatibility exception |
| Android, default mode | Zig tools | Zig | `check` works; Android libc linking is currently unavailable in Zig |
| Android with `-ndkfallback` | NDK Clang and LLVM ar | NDK LLD | explicit fallback; not Zig-linked |

Zirild never switches an Android build to NDK Clang/LLD silently. The fallback
requires `-ndkfallback` and emits a prominent build warning.

## Install

```powershell
cargo install cargo-zirild
```

Set `ZIG_HOME` to the Zig installation directory or directly to `zig`/`zig.exe`:

```powershell
$env:ZIG_HOME = 'C:\tools\zig'
```

Zig lookup order is:

1. `--zig-path=<path>`;
2. `CARGO_ZIRILD_ZIG_PATH`, `ZIG`, `ZIG_HOME`, then legacy `Zig_home`;
3. `zig` from `PATH`.

The legacy `-zigpatch=<path>` spelling remains accepted. Zirild validates the
selected executable with `zig version` before Cargo starts.

The selected Rust target must also be installed in the active Rust toolchain:

```powershell
rustup target add x86_64-pc-windows-gnu
```

## Quick start

```powershell
# build is the default Cargo command
cargo zirild -target=x86_64-windows-gnu

# Build a Linux GNU target using an explicit Zig executable.
cargo zirild -target=x86_64-unknown-linux-gnu --zig-path=D:\tools\zig\zig.exe

# Build and run a native Windows MSVC target.
cargo zirild -target=x86_64-pc-windows-msvc run

# Pass arguments to the produced executable.
cargo zirild -target=x86_64-pc-windows-msvc run -- --example-argument
```

Both Zig target names and recognized Rust target triples are accepted. For
example, `x86_64-windows-gnu` maps to `x86_64-pc-windows-gnu`, while the short
`x86_64-pc-windows` form selects `x86_64-pc-windows-msvc`.

Do not pass Cargo's `--target`; Zirild's `-target` is the single source of truth
and is converted to the corresponding Rust target triple.

## Command reference

```text
cargo zirild -target=<zig-target> [zirild options] [cargo command] [cargo options]
```

Supported Cargo commands are `build` (default), `check`, `run`, `test`,
`bench`, `rustc`, `clippy`, and `doc`. Other arguments, including `--package`,
`--bin`, `--features`, `--locked`, and arguments after `--`, are passed to the
selected Cargo command.

When selecting a command explicitly, place it before its Cargo options:

```powershell
cargo zirild -target=x86_64-pc-windows-msvc run --release -- --example-argument
```

After the command, Zirild no longer guesses whether an argument value such as
`run` is another command. If the command is omitted, the first non-Zirild
argument starts the default `build` arguments. Cargo's `-Z <feature>` is passed
through unchanged; Zirild does not reserve `-Z`.

### General options

| Option | Meaning |
| --- | --- |
| `-target=<target>` | required Zig target or recognized Rust target triple |
| `--zig-path=<path>` | select a Zig directory or executable for this invocation |
| `--release` | Cargo release profile and Zig `ReleaseFast` |
| `--zig-release-small` | select Zig `ReleaseSmall` and force native `-Oz` |
| `--zig-opt=<mode>` | select `debug`, `safe`, `fast`, `small`, or a full Zig mode name |
| `--windows-runtime=<policy>` | Windows GNU runtime policy: `auto`, `zig`, or `preserve` |
| `--preserve-linker-args` | alias for `--windows-runtime=preserve` |
| `--trace` | print native wrapper invocation details |
| `-h`, `--help` | show command help |

The legacy `-zigpatch=<path>` and `-ZigOptimize=<mode>` spellings remain
accepted for compatibility.

The default link policy is Zig `ReleaseSafe`; Cargo `--release` maps the link
policy to `ReleaseFast`. Native C/C++ compilation normally keeps the
optimization flags selected by `cc-rs`, CMake, or another build helper from
Cargo's profile environment. An explicit `--zig-opt` or
`--zig-release-small` removes conflicting native `-O*` arguments and appends
the selected `-O0`, `-O2`, `-O3`, or `-Oz` flag last. Rust crates always retain
Cargo's normal profile semantics.

## Android NDK

Set `ANDROID_NDK_HOME`, `ANDROID_NDK_ROOT`, or legacy `Ndk_home` to either:

- one NDK installation; or
- the Android SDK `ndk` directory containing version subdirectories.

When multiple valid NDK versions exist, Zirild selects the newest version. It
prints the selected version and full path in every Android build log.

```powershell
$env:ANDROID_NDK_HOME = 'D:\Android\Sdk\ndk'

# Use the newest installed NDK.
cargo zirild -target=x86_64-linux-android check

# Select an exact installed version.
cargo zirild -target=x86_64-linux-android -ndkversion='27.2.12479018' check

# Select by newest-first index: 0 is newest, 1 is the next oldest.
cargo zirild -target=x86_64-linux-android -nindx=1 check
```

### Android options

| Option | Meaning |
| --- | --- |
| `--ndk-path=<path>` | select one NDK or an SDK `ndk` directory for this invocation |
| `-ndkversion=<version>` | select an exact version below the NDK search directory |
| `-nindx=<index>` | select from the newest-first version list |
| `-android-api=<level>` | select the Android API level |
| `-android-abi=<abi>` | validate the ABI, such as `arm64-v8a` or `x86_64` |
| `-android-stl=<name>` | export the requested NDK STL setting |
| `-android-cflag=<flag>` | pass one flag to the selected native compiler |
| `-android-link-arg=<flag>` | pass one flag to the selected native linker |
| `-ndkfallback` | explicitly use NDK Clang/LLD and LLVM ar instead of Zig |

CMake-style `-DANDROID_ABI=...`,
`-DANDROID_PLATFORM=android-<api>`, and `-DANDROID_STL=...` aliases are also
parsed in Android mode. Zirild exports their values for Cargo build scripts,
along with `ANDROID_NDK_ROOT` and `CMAKE_ANDROID_NDK`.

NDK lookup order is `--ndk-path`, `CARGO_ZIRILD_NDK_PATH`,
`ANDROID_NDK_HOME`, `ANDROID_NDK_ROOT`, legacy `Ndk_home`, then the `ndk`
directory below `ANDROID_SDK_ROOT` or `ANDROID_HOME`.

Example native configuration:

```powershell
cargo zirild -target=x86_64-linux-android `
  -android-api=24 `
  -android-abi=x86_64 `
  -android-cflag=-fPIC `
  -android-link-arg=-Wl,--build-id `
  check
```

### Android fallback

The current Zig distribution cannot link Android libc even when an NDK sysroot
is available. Default Android mode therefore preserves the Zig-linker contract
and reports the limitation instead of silently producing a file with another
linker.

To produce an Android file with the installed NDK, opt in explicitly:

```powershell
cargo zirild -target=x86_64-linux-android -ndkfallback build
```

This switches C, C++, archive, and final-link steps to NDK Clang/LLD and LLVM
ar. The build log warns that the final output is not linked by Zig.

A suggested musl target is only a compatibility alternative: it produces a
self-contained Linux binary, not an Android application.

## Platform notes

### Linux GNU

Both target spellings below select the same Cargo target:

```powershell
rustup target add x86_64-unknown-linux-gnu
cargo zirild -target=x86_64-linux-gnu
cargo zirild -target=x86_64-unknown-linux-gnu
```

Zig supplies the GNU libc toolchain for ordinary Rust and native C/C++
dependencies. Applications using desktop libraries outside libc, such as DBus,
GTK, X11, or WebKit2GTK, additionally need a target sysroot containing those
libraries and their pkg-config metadata.

### Windows

Windows GNU uses Zig for C, C++, archives, raw-dylib import libraries, and final
linking. Zirild removes Rust's ignored `--enable/--disable-auto-image-base`
switches before Zig LLD is invoked. Linker response files are rewritten into a
private wrapper directory, and empty module-definition files are copied there
before a fallback export is added. Zirild never edits rustc's original `.rsp`
or `.def` inputs in place.

The default `--windows-runtime=auto` policy replaces the known Rust MinGW
runtime arguments with Zig's bundled runtime and adds `-lunwind`. It warns when
custom startup or entry-point arguments are detected. Use `zig` to acknowledge
and force that policy, or `preserve` for `no_std`, a custom CRT, custom startup
objects, or manually controlled unwind libraries. `preserve` disables runtime
argument removal and the automatic `-lunwind` addition.

Windows MSVC currently retains the system compiler, librarian, and linker for
ABI compatibility. It is a documented exception to the unified Zig-linker
goal.

## Validation

The current source was validated on 2026-07-14 with a temporary standalone
fixture outside the published package.

| Rust target | Mode and linker | Result |
| --- | --- | --- |
| `aarch64-linux-android` | `-ndkfallback`, NDK 30.0.14904198 Clang/LLD | passed; ELF64 AArch64 PIE |
| `x86_64-linux-android` | `-ndkfallback`, NDK 30.0.14904198 Clang/LLD | passed; ELF64 x86-64 PIE |
| `x86_64-unknown-linux-gnu` | Zig | passed; mixed Rust/C/C++ ELF ran in Ubuntu WSL2 |
| `x86_64-pc-windows-gnu` | Zig | passed; mixed Rust/C/C++ PE ran on Windows |
| `x86_64-pc-windows-msvc` | system MSVC | passed; mixed Rust/C/C++ PE ran on Windows |

The native fixture used `cc-rs`, one C translation unit, one C++17 translation
unit, and Rust `extern "C"` calls. Build traces confirmed the expected C, C++,
archive, and final-link wrapper modes. All three runnable binaries completed
with exit code 0 and produced the expected interop results.

Android artifacts were inspected with the selected NDK's `llvm-readelf`.
Android device execution has not yet been validated.

The latest reliability pass also verified that Windows linker source files
remain unchanged, temporary wrapper files are removed after a successful
build, Cargo nightly `-Z` arguments remain intact, and Android NDK discovery
works through `ANDROID_NDK_HOME`.

## Scope and current limits

Zirild is currently strongest as a Windows-hosted, native-dependency-aware
Cargo driver for Windows GNU and Linux targets. It is not yet a universal SDK
or build-system replacement.

- Windows MSVC is orchestrated by Zirild but compiled and linked by MSVC.
- Strict Zig Android mode can check code but cannot currently link Android libc;
  `-ndkfallback` is explicit NDK LLVM orchestration.
- Android device execution is not yet covered by the validation matrix.
- Minimum-glibc suffixes, Apple universal binaries, custom target JSON, CMake
  toolchain generation, bindgen sysroots, and pkg-config sysroot management are
  not yet first-class Zirild features.
- The mixed Rust/C/C++ fixture covers `cc-rs`, archives, and final linking, but
  does not claim exhaustive compatibility with OpenSSL, `ring`, large CMake
  projects, Tauri Android, or every custom-CRT/`no_std` layout.

## Output paths and execution

Artifacts use Cargo's normal locations:

- `target/<cargo-target>/debug/` by default;
- `target/<cargo-target>/release/` with `--release`.

`run` and `test` require the host to execute the selected target. Use `build`
or `check` for foreign targets unless a compatible runtime is available.

## License

This project is licensed under the BSD 2-Clause License.

Copyright (c) 2026 SorMaze.

Redistribution in source or binary form is permitted provided that the
copyright notice, conditions, and disclaimer in [LICENSE](LICENSE) are
retained.
