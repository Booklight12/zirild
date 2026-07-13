#include <cstdio>

extern "C" int meaning_from_c(void);

int main() {
    std::printf("C and C++ agree: %d\n", meaning_from_c());
    return 0;
}
