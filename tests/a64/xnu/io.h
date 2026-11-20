/* Output helpers of the dynamically linked arm64 test programs: they write
 * through the libSystem stand-in's `poly_write` (ADR 0257). */
#ifndef POLY_A64_XNU_IO_H
#define POLY_A64_XNU_IO_H

extern long poly_write(int fd, const char *buf, long len);

static inline unsigned long io_strlen(const char *s) {
    unsigned long n = 0;
    while (s[n]) n++;
    return n;
}

static inline void say(const char *s) { poly_write(1, s, (long)io_strlen(s)); }

static inline void say_num(unsigned long v) {
    char t[24];
    int n = 0;
    do {
        t[n++] = (char)('0' + v % 10);
        v /= 10;
    } while (v);
    char out[24];
    for (int i = 0; i < n; i++) out[i] = t[n - 1 - i];
    out[n] = 0;
    say(out);
}

static inline void say_hex(unsigned long v) {
    char out[19] = "0x";
    for (int i = 0; i < 16; i++) out[2 + i] = "0123456789abcdef"[(v >> (60 - 4 * i)) & 15];
    out[18] = 0;
    say(out);
}

#endif
