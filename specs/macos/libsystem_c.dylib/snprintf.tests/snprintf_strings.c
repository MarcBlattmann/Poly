/* snprintf: %s, %c, %p and the wide forms; a NULL string prints (null);
 * wide characters above 0xFF are an error (EILSEQ) in the C locale. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <errno.h>
#include <stdio.h>
#include <string.h>
#include <wchar.h>

static int (*volatile sn)(char *, size_t, const char *, ...) = snprintf;
/* One call into a 128 byte buffer: the name, the return value and the text. */
#define CK(want_ret, want_str, ...) do { \
    char b_[128]; memset(b_, 0, sizeof b_); \
    int r_ = sn(b_, sizeof b_, __VA_ARGS__); \
    check_str("text of " #__VA_ARGS__, b_, want_str); \
    check("return of " #__VA_ARGS__, r_ == (want_ret), r_); \
} while (0)

int test_main(void) {
    CK(3, "abc", "%s", "abc");
    CK(0, "", "%s", "");
    CK(7, "abc   |", "%-6s|", "abc");
    CK(7, "   abc|", "%6s|", "abc");
    CK(1, "a", "%.1s", "abc");
    CK(0, "", "%.0s", "abc");
    CK(2, "ab", "%.*s", 2, "abcdef");
    CK(2, "ab", "%.100s", "ab");
    CK(7, "000abc|", "%06s|", "abc");
    CK(6, "(null)", "%s", (char *)0);
    CK(2, "(n", "%.2s", (char *)0);
    CK(11, "    (null)|", "%10s|", (char *)0);
    CK(2, "\xc3\xa9", "%s", "\xc3\xa9");
    CK(1, "\xc3", "%.1s", "\xc3\xa9");
    CK(6, "    a|", "%5c|", 'a');
    CK(1, "A", "%C", 65);
    CK(1, "B", "%lc", 66);
    CK(4, "wide", "%S", L"wide");
    CK(4, "wide", "%ls", L"wide");
    CK(2, "wi", "%.2ls", L"wide");
    CK(1, "\xe9", "%lc", (wint_t)0xe9);
    CK(3, "0x0", "%p", (void *)0);
    CK(4, "0x10", "%p", (void *)0x10);
    CK(4, "0x10", "%#p", (void *)0x10);
    CK(8, "0xdeadbe", "%p", (void *)0xdeadbe);
    CK(3, "a:b", "%s:%s", "a", "b");
    CK(2, "ac", "%s%s%s", "a", "", "c");

    char b[32];
    memset(b, 'x', sizeof b);
    errno = 0;
    int r = sn(b, sizeof b, "%lc", (wint_t)0x100);
    check("%lc of 0x100: -1", r == -1, r);
    check("%lc of 0x100: EILSEQ (92)", errno == EILSEQ && errno == 92, errno);
    errno = 0;
    r = sn(b, sizeof b, "%ls", L"\x100");
    check("%ls with 0x100: -1", r == -1 && errno == 92, errno);
    errno = 0;
    r = sn(b, sizeof b, "%lc", (wint_t)0x7fffffff);
    check("%lc of 0x7fffffff: -1 EILSEQ", r == -1 && errno == 92, errno);

    memset(b, 'x', sizeof b);
    r = sn(b, sizeof b, "%lc", (wint_t)0);
    check("%lc of 0: counts one character", r == 1, r);
    check("the NUL is stored", b[0] == 0, b[0]);
    return finish();
}
