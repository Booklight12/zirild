# Changelog

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
