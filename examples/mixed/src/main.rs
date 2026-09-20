//! Rust + C + C++17 interop fixture for `cargo-zirild`.
//!
//! Build it the same way a real project would be built, so the wrapper modes for
//! C compilation, C++ compilation, archiving, and the final link are all used:
//!
//! ```text
//! cargo zirild -target=x86_64-pc-windows-gnu run
//! cargo zirild -target=x86_64-unknown-linux-gnu build
//! cargo zirild -target=x86_64-unknown-linux-musl run
//! ```
//!
//! The expected interop result is C = 42 and C++ = 43.

use std::process::ExitCode;

unsafe extern "C" {
    fn meaning_from_c() -> i32;
    fn meaning_from_cpp() -> i32;
}

fn main() -> ExitCode {
    // SAFETY: both symbols are provided by the archives built from `support.c`
    // and `main.cpp`, and both take no arguments and return a plain `int`.
    let from_c = unsafe { meaning_from_c() };
    let from_cpp = unsafe { meaning_from_cpp() };
    println!("Rust received {from_c} from C and {from_cpp} from C++");
    if from_c == 42 && from_cpp == 43 {
        println!("mixed Rust/C/C++ interop ok");
        ExitCode::SUCCESS
    } else {
        eprintln!("mixed Rust/C/C++ interop failed: C = {from_c}, C++ = {from_cpp}");
        ExitCode::FAILURE
    }
}
