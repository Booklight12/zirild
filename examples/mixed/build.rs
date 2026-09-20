fn main() {
    // The C and C++ translation units must become separate archives: `cc` applies
    // `.cpp(true)` to every file in one `Build`, and compiling `support.c` as C++
    // would give `meaning_from_c` C++ linkage while `main.cpp` declares it
    // `extern "C"`. The C archive is compiled first so it precedes the C++
    // archive on rustc's link line; the Rust caller references `meaning_from_c`
    // directly, so the linker pulls that member before it reaches the C++
    // archive that also needs it.
    cc::Build::new()
        .file("support.c")
        .compile("zirild_mixed_support");

    cc::Build::new()
        .cpp(true)
        .file("main.cpp")
        .flag_if_supported("-std=c++17")
        .compile("zirild_mixed_bridge");

    println!("cargo:rerun-if-changed=support.c");
    println!("cargo:rerun-if-changed=main.cpp");
}
