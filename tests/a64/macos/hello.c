#include "rt.h"
int run(void) {
    puts_("hello from arm64\n");
    i64 s = 0;
    for (int i = 1; i <= 100; i++) s += i * i;
    puts_("sum of squares 1..100 = "); puti(s); nl();
    return 0;
}
