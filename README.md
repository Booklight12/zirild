# cargo-zirild

[Repository](https://github.com/Booklight12/zirild)

cargo-zirild builds the current Rust Cargo project and its dependencies for
Zig-supported targets. It creates temporary tool wrappers automatically; no
.cargo/config.toml, CC, CXX, linker, AR, or dlltool configuration is required
from the caller.

Rust source remains compiled by rustc. For GNU-family targets, Rust linking
uses zig cc, while cc-crate build scripts use zig cc, zig c++, and zig ar for
C, C++, and archives respectively. Windows GNU raw-dylib import libraries use
zig dlltool. MSVC targets retain the system MSVC compiler, linker, and
librarian because MSVC build scripts require that ABI-compatible tool family.

## Prerequisites

Set Zig_home to the Zig installation directory or to the full zig/zig.exe
path. Use -zigpatch=<path> to override Zig_home for one command; the override
can be a Zig directory or executable and is verified with zig version before
Cargo starts. The selected Rust target must be installed in the active Rust toolchain,
because Cargo/rustc provides the Rust standard library for that target.

The tested Zig 0.17-dev toolchain has no bundled Android libc. A direct Android
NDK target needs an Android NDK sysroot, because Rust's Android standard
library deliberately does not bundle system libraries such as liblog. For a
musl-based Android-compatible native build, use the corresponding musl target,
such as x86_64-linux-musl; this is a musl Linux artifact, not an
Android-NDK-linked artifact.

    $env:Zig_home = 'C:\tools\zig'
    rustup target add x86_64-pc-windows-gnu

## Build

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

The resulting artifact uses Cargo's normal path:
target/<cargo-target>/debug/ by default or target/<cargo-target>/release/ with
--release.

## Linux GNU

Both Zig and Rust target spellings are accepted:

    rustup target add x86_64-unknown-linux-gnu
    cargo zirild -target=x86_64-linux-gnu
    cargo zirild -target=x86_64-unknown-linux-gnu

Zig supplies the GNU libc toolchain for ordinary Rust and native C/C++
dependencies. Applications that link Linux desktop libraries outside libc,
such as DBus, GTK, X11, or WebKit2GTK, additionally require a target Linux
sysroot containing those libraries and their pkg-config metadata. Zig does not
bundle application-level Linux desktop libraries.

## Zig optimization mapping

| Command form | Zig mode |
| --- | --- |
| default (Cargo dev) | ReleaseSafe |
| --release | ReleaseFast |
| -Z | ReleaseSmall |
| -ZigOptimize=<mode> | explicit Debug, ReleaseSafe, ReleaseFast, or ReleaseSmall |

All other arguments are passed through to cargo build, including --package,
--bin, --features, and --locked.

For C/C++ compilation and Zig linking, these policies are translated to the
frontend flags -O0, -O2, -O3, and -Oz respectively. Rust crates themselves
keep Cargo's normal dev/release profile semantics.

When an Android target is requested, cargo-zirild reports that the bundled Zig
toolchain has no Android libc and suggests the corresponding musl target. A
musl binary may be suitable for Android-like environments, but it is not an
Android-NDK-linked artifact.

## License

This project is licensed under the BSD 2-Clause License.

Copyright (c) 2026 SorMaze.

Redistribution in source or binary form is permitted provided that the
copyright notice, conditions, and disclaimer in [LICENSE](LICENSE) are
retained.
