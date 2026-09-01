#include <stdio.h>
#include <stdint.h>

long long println_i64(long long value) {
    printf("%lld\n", (long long)value);
    return 0;
}

long long println_str(const char *value) {
    printf("%s\n", value);
    return 0;
}
