# cargo-zirild

[GitHub repository](https://github.com/Booklight12/zirild)

`cargo-zirild` is a Cargo subcommand for building, checking, testing, and
running the current Rust project for a selected target. It configures temporary
Zig tool wrappers automatically; callers do not need to create
`.cargo/config.toml` or set `CC`, `CXX`, linker, `AR`, or `dlltool` variables.

Rust source is still compiled by `rustc`. For GNU-family targets, Rust linking
uses `zig cc`, while `cc` crate build scripts use `zig cc`, `zig c++`, and
`zig ar` for C, C++, and archives. Windows GNU raw-dylib import libraries use
`zig dlltool`. MSVC targets retain the system MSVC compiler, linker, and
librarian because their build scripts require that ABI-compatible tool family.

## Installation

```powershell
cargo install cargo-zirild
```

## Prerequisites

Set `Zig_home` to the Zig installation directory or the full `zig`/`zig.exe`
path. Use `-zigpatch=<path>` to override `Zig_home` for one command. Zirild
accepts either a Zig directory or executable and verifies it with `zig version`
before Cargo starts.

The selected Rust target must be installed in the active Rust toolchain because
Cargo and `rustc` provide the Rust standard library for that target.

For Android target discovery, set `Ndk_home` to an NDK installation or to the
Android SDK `ndk` directory. When it points to an SDK directory containing
multiple versions, Zirild selects the newest valid NDK installation (for
example, it selects `30.0.14904198` over `27.2.12479018`). Use
`-ndkversion=<version>` to select an exact existing version, or
`-nindx=<index>` to select from the newest-first ordered list (`0` is newest,
`1` is the next oldest). Zirild prints the selected version and full path for
every Android Cargo invocation.

Zirild's contract is that Zig remains the C/C++ compiler and final linker for
every non-MSVC target. The current Zig distribution cannot link Android libc,
even when an NDK sysroot exists, so Zirild deliberately does not substitute
NDK clang for its final link. Android `check` can succeed; Android native
`build` currently cannot produce an NDK-linked file through Zig. Zirild reports
the discovered NDK and the constraint rather than silently producing a file
with a different linker. It suggests the corresponding musl target as a
compatibility alternative; that alternative produces a self-contained musl
Linux binary, not an Android-NDK-linked application.

If producing an Android file is more important than retaining the Zig-linker
contract, pass `-ndkfallback`. This is
an explicit opt-in to the selected NDK's Clang/LLD and LLVM ar. Zirild prints a
warning before the Cargo command because the final file is then **not** linked
by Zig.

```powershell
$env:Zig_home = 'C:\tools\zig'
rustup target add x86_64-pc-windows-gnu
```

## Cargo commands

```powershell
# Zig target x86_64-windows-gnu maps to Cargo x86_64-pc-windows-gnu.
cargo zirild -target=x86_64-windows-gnu

# Short Windows target selects x86_64-pc-windows-msvc.
cargo zirild -target=x86_64-pc-windows

cargo zirild -target=x86_64-unknown-linux-gnu -zigpatch=D:\tools\zig\zig.exe

# Cargo release profile and Zig ReleaseFast native/link optimization.
cargo zirild -target=aarch64-linux-musl --release

# Zig ReleaseSmall.
cargo zirild -target=aarch64-linux-musl -Z

# Override only Zig's optimization mode.
cargo zirild -target=x86_64-linux-gnu -ZigOptimize=ReleaseSafe --features tls
```

## Android NDK options

Android settings are consumed by Zirild rather than forwarded as invalid Cargo
arguments. The selected NDK root/sysroot are exported to build scripts, while
explicit native flags are given to the Zig compiler/linker wrappers; Zig remains
the final linker.

```powershell
# Default: select the newest valid NDK under $env:Ndk_home.
cargo zirild -target=x86_64-linux-android check

# Select a particular installed NDK version. Quote dotted versions in PowerShell.
cargo zirild -target=x86_64-linux-android -ndkversion='27.2.12479018' check

# Newest-first index: 0 is newest, 1 is the next oldest.
cargo zirild -target=x86_64-linux-android -nindx=1 check

# Android API and ABI; CMake-style ANDROID_* forms are accepted as aliases.
cargo zirild -target=x86_64-linux-android -android-api=24 -android-abi=x86_64 check
cargo zirild -target=x86_64-linux-android -DANDROID_PLATFORM=android-24 -DANDROID_ABI=x86_64 check

# Pass native flags only to Zig's C/C++ or final-linker invocation.
cargo zirild -target=x86_64-linux-android -android-cflag=-fPIC -android-link-arg=-Wl,--build-id check

# Explicitly fall back to the selected NDK Clang/LLD to produce an Android file.
# The final linker is not Zig; Zirild prints a warning.
cargo zirild -target=x86_64-linux-android -ndkfallback build
```

Supported Android-specific options are `-ndkversion`, `-nindx`,
`-android-api`, `-android-abi`, `-android-stl`, `-android-cflag`, and
`-android-link-arg`. Zirild also parses CMake-style `-DANDROID_ABI=...`,
`-DANDROID_PLATFORM=android-<api>`, and `-DANDROID_STL=...`; they become
`ANDROID_ABI`, `ANDROID_PLATFORM`, and `ANDROID_STL` environment values for
Cargo build scripts. It also exports the selected NDK as `ANDROID_NDK_ROOT` and
`CMAKE_ANDROID_NDK`. `-ndkfallback` is Android-only and intentionally switches
the C compiler, C++ compiler, linker, and archiver to NDK Clang/LLD and LLVM ar.

## Current target validation

The following matrix was compiled on Windows against the locally installed Rust
targets on 2026-07-14. The validation project was
`D:\User\Document\RustProject\t1`, a standalone Rust binary with no native
dependencies. This validates compilation and linker output, not execution on a
device or foreign operating system.

| Rust target | Zirild invocation | Native linker | Result |
| --- | --- | --- | --- |
| `aarch64-linux-android` | `-ndkfallback build` | NDK 30.0.14904198 Clang/LLD | passed; ELF64 AArch64 PIE |
| `x86_64-linux-android` | `-ndkfallback build` | NDK 30.0.14904198 Clang/LLD | passed; ELF64 x86-64 PIE |
| `x86_64-pc-windows-gnu` | `build` | Zig LLD | passed |
| `x86_64-pc-windows-msvc` | `build` | system MSVC | passed |
| `x86_64-unknown-linux-gnu` | `build` | Zig LLD | passed |

The Android artifacts were inspected with the selected NDK's `llvm-readelf`.
For Windows GNU, Zirild filters Rust's `--enable/--disable-auto-image-base`
switches because Zig LLD reports them as unimplemented and ignores them; this
removes the warning without changing Zig's image-base behavior.

`build` is the default Cargo command. You can also select `check`, `run`,
`test`, `bench`, `rustc`, `clippy`, or `doc`. The command may follow the Zirild
options, so a native Windows MSVC binary can be built and started with:

```powershell
cargo zirild -target=x86_64-pc-windows-msvc run
```

Arguments after `--` are preserved for commands that execute a binary:

```powershell
cargo zirild -target=x86_64-pc-windows-msvc run -- --example-argument
```

Cross-compiled executables can only be run when the host OS can execute the
selected target. Use the default `build` or `check` command for other targets.

Commands that produce artifacts use Cargo's normal paths:
`target/<cargo-target>/debug/` by default, or
`target/<cargo-target>/release/` with `--release`.

## Linux GNU

Both Zig and Rust target spellings are accepted:

```powershell
rustup target add x86_64-unknown-linux-gnu
cargo zirild -target=x86_64-linux-gnu
cargo zirild -target=x86_64-unknown-linux-gnu
```

Zig supplies the GNU libc toolchain for ordinary Rust and native C/C++
dependencies. Applications that link Linux desktop libraries outside libc,
such as DBus, GTK, X11, or WebKit2GTK, additionally require a target Linux
sysroot containing those libraries and their pkg-config metadata. Zig does not
bundle application-level Linux desktop libraries.

## Zig optimization mapping

| Command form | Zig mode |
| --- | --- |
| default (Cargo dev) | `ReleaseSafe` |
| `--release` | `ReleaseFast` |
| `-Z` | `ReleaseSmall` |
| `-ZigOptimize=<mode>` | explicit `Debug`, `ReleaseSafe`, `ReleaseFast`, or `ReleaseSmall` |

All other arguments are passed through to the selected Cargo command,
including `--package`, `--bin`, `--features`, `--locked`, and arguments after
`--`.

For C/C++ compilation and Zig linking, these policies are translated to the
frontend flags `-O0`, `-O2`, `-O3`, and `-Oz`, respectively. Rust crates
themselves keep Cargo's normal dev/release profile semantics.

When an Android target is requested, `cargo-zirild` looks for `Ndk_home`,
reports the selected NDK during the actual Cargo invocation, exports its root
and sysroot for build scripts, forwards Android-only native flags to Zig, and
retains Zig as the final linker. It also explains the current Android libc
linkage constraint. The musl suggestion is an explicit compatibility
alternative, not an Android target. With explicit `-ndkfallback`, Zirild instead
uses NDK Clang/LLD and clearly warns that the final linker is no longer Zig.

## License

This project is licensed under the BSD 2-Clause License.

Copyright (c) 2026 SorMaze.

Redistribution in source or binary form is permitted provided that the
copyright notice, conditions, and disclaimer in [LICENSE](LICENSE) are
retained.
