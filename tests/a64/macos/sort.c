#include "rt.h"
static u64 rng = 88172645463325252ull;
static u64 next(void) { rng ^= rng << 13; rng ^= rng >> 7; rng ^= rng << 17; return rng; }
static void isort(i32 *a, int n) {
    for (int i = 1; i < n; i++) { i32 v = a[i]; int j = i - 1; while (j >= 0 && a[j] > v) { a[j + 1] = a[j]; j--; } a[j + 1] = v; }
}
static void qs(i64 *a, int lo, int hi) {
    while (lo < hi) {
        i64 p = a[(lo + hi) / 2]; int i = lo, j = hi;
        while (i <= j) { while (a[i] < p) i++; while (a[j] > p) j--; if (i <= j) { i64 t = a[i]; a[i] = a[j]; a[j] = t; i++; j--; } }
        if (j - lo < hi - i) { qs(a, lo, j); lo = i; } else { qs(a, i, hi); hi = j; }
    }
}
static int cmpstr(const char *a, const char *b) { while (*a && *a == *b) { a++; b++; } return (u8)*a - (u8)*b; }
static i64 big[20000];
static i32 small[500];
int run(void) {
    for (int i = 0; i < 20000; i++) big[i] = (i64)next();
    qs(big, 0, 19999);
    int ok = 1; for (int i = 1; i < 20000; i++) ok &= big[i - 1] <= big[i];
    puts_("quicksort ok="); puti(ok); puts_(" min="); puti(big[0]); puts_(" max="); puti(big[19999]); nl();
    for (int i = 0; i < 500; i++) small[i] = (i32)(next() % 1000) - 500;
    isort(small, 500);
    i64 chk = 0; for (int i = 0; i < 500; i++) chk = chk * 31 + small[i];
    puts_("insertion sort checksum="); puti(chk); nl();
    const char *words[] = {"pear", "apple", "fig", "banana", "cherry", "date", "apricot", "kiwi"};
    for (int i = 0; i < 8; i++) for (int j = i + 1; j < 8; j++) if (cmpstr(words[j], words[i]) < 0) { const char *t = words[i]; words[i] = words[j]; words[j] = t; }
    for (int i = 0; i < 8; i++) { puts_(words[i]); putc_(' '); } nl();
    return 0;
}
