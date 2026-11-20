#include "rt.h"
static size_t slen(const char *s) { size_t n = 0; while (s[n]) n++; return n; }
static const char B64[] = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
static size_t b64(const u8 *in, size_t n, char *out) {
    size_t o = 0;
    for (size_t i = 0; i < n; i += 3) {
        u32 v = (u32)in[i] << 16 | (i + 1 < n ? (u32)in[i+1] << 8 : 0) | (i + 2 < n ? in[i+2] : 0);
        out[o++] = B64[v >> 18]; out[o++] = B64[(v >> 12) & 63];
        out[o++] = i + 1 < n ? B64[(v >> 6) & 63] : '='; out[o++] = i + 2 < n ? B64[v & 63] : '=';
    }
    out[o] = 0; return o;
}
static int utf8_count(const u8 *s) { int n = 0; while (*s) { if ((*s & 0xC0) != 0x80) n++; s++; } return n; }
static void reverse(char *s, size_t n) { for (size_t i = 0; i < n / 2; i++) { char t = s[i]; s[i] = s[n-1-i]; s[n-1-i] = t; } }
static int count_words(const char *s) { int n = 0, in = 0; for (; *s; s++) { int sp = *s == ' ' || *s == '\n'; if (!sp && !in) n++; in = !sp; } return n; }
static char upper(char c) { return c >= 'a' && c <= 'z' ? c - 32 : c; }
static char buf[8192], buf2[8192];
static u8 big[65536];
int run(void) {
    const char *t = "The quick brown fox jumps over the lazy dog";
    char e[128]; b64((const u8 *)t, slen(t), e); puts_(e); nl();
    memcpy(buf, t, slen(t) + 1); reverse(buf, slen(buf)); puts_(buf); nl();
    for (size_t i = 0; buf[i]; i++) buf[i] = upper(buf[i]); puts_(buf); nl();
    puts_("words="); puti(count_words(t)); puts_(" utf8 chars="); puti(utf8_count((const u8 *)"gr\xc3\xbc\xc3\x9f dich \xe2\x82\xac")); nl();
    for (int i = 0; i < 65536; i++) big[i] = (u8)(i ^ (i >> 7));
    u64 hist[256] = {0}; for (int i = 0; i < 65536; i++) hist[big[i]]++;
    u64 h = 0; for (int i = 0; i < 256; i++) h = h * 1000003 + hist[i];
    puts_("histogram hash="); putx(h); nl();
    memmove(big + 3, big, 60000); memset(big + 100, 0x5A, 1000);
    u64 s = 0; for (int i = 0; i < 65536; i++) s += big[i] * (u64)(i | 1);
    puts_("memmove sum="); putu(s); nl();
    int n = 0; for (int i = 0; i < 300; i++) { const char *w = (i % 3) ? "ab" : "xyz"; while (*w) buf2[n++] = *w++; } buf2[n] = 0;
    int cnt = 0; for (int i = 0; buf2[i]; i++) if (buf2[i] == 'x' && buf2[i+1] == 'y') cnt++;
    puts_("pattern count="); puti(cnt); puts_(" len="); puti(n); nl();
    return 0;
}
