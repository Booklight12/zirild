# cargo-zirild

`cargo-zirild` uses Zig as a cross-platform C/C++ compiler and linker.  It is
installed as a Cargo subcommand, so it is invoked as `cargo zirild`.

## Prerequisites

Set `Zig_home` to the Zig installation directory or the full path to its
executable.  The directory must contain `zig.exe` on Windows (or `zig` on
other platforms).

```powershell
$env:Zig_home = 'C:\tools\zig'
```

## Install and use

```powershell
cargo install --path .
cargo zirild --target x86_64-windows-gnu --output .\build\hello.exe .\src\hello.c
cargo zirild --target aarch64-linux-musl --output .\build\hello .\src\hello.cpp
```

The tool compiles each `.c`, `.cc`, `.cpp`, `.cxx`, or `.c++` source to an
object with `zig cc` or `zig c++`, then links them with `zig c++` when any C++
source is present (otherwise `zig cc`).  `--target` is passed directly to
Zig, so any target Zig supports can be selected.

Use `cargo zirild --help` for include paths, definitions, libraries, and
per-language or linker flags.

## Mixed C/C++ example

```powershell
cargo zirild --target x86_64-windows-gnu --output .\build\mixed.exe `
  .\examples\mixed\main.cpp .\examples\mixed\support.c
```
