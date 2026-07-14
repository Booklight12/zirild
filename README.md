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

Zig alone does not provide an Android NDK sysroot. Android target checking can
succeed, but linking an Android executable requires NDK libraries such as
`liblog`. Zirild suggests the corresponding musl target when Android is
requested. That alternative produces a self-contained musl Linux binary, not
an Android-NDK-linked application.

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

When an Android target is requested, `cargo-zirild` reports the missing Android
libc/sysroot support and suggests the corresponding musl Linux target. The
suggestion is an explicit compatibility alternative, not an Android target.

## License

This project is licensed under the BSD 2-Clause License.

Copyright (c) 2026 SorMaze.

Redistribution in source or binary form is permitted provided that the
copyright notice, conditions, and disclaimer in [LICENSE](LICENSE) are
retained.
