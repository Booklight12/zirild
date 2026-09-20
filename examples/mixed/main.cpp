// The C++ translation unit exports one C-linkage entry point instead of owning
// `main`: the fixture is driven from Rust, which declares both native functions
// as `extern "C"`. The C++ side still performs a real cross-language call back
// into the C unit, so the fixture exercises C compilation, C++ compilation, both
// archives, and the final link.
#include <cstdio>

extern "C" int meaning_from_c(void);

extern "C" int meaning_from_cpp(void) {
    const int from_c = meaning_from_c();
    std::printf("C++ received %d from C\n", from_c);
    return from_c + 1;
}
