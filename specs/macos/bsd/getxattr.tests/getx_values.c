#include "poly_mtest.h"
#define SYS_getxattr 234
#define SYS_fgetxattr 235
#define SYS_setxattr 236
#define SYS_fsetxattr 237
#define SYS_removexattr 238
#define SYS_fremovexattr 239
#define SYS_listxattr 240
#define SYS_flistxattr 241
#define ENOATTR 93
#define SYS_rename 128
#define SYS_link 9
#define SYS_ftruncate 201
#define XATTR_NOFOLLOW 1
#define XATTR_CREATE 2
#define XATTR_REPLACE 4
#define O_SYMLINK 0x200000
static __attribute__((unused)) sysret gx(const char *p, const char *n, void *b, i64 sz, i64 pos, i64 opt) { return bsd6(SYS_getxattr, p, n, b, sz, pos, opt); }
static __attribute__((unused)) sysret sx(const char *p, const char *n, const void *b, i64 sz, i64 pos, i64 opt) { return bsd6(SYS_setxattr, p, n, b, sz, pos, opt); }
static __attribute__((unused)) sysret fgx(i64 fd, const char *n, void *b, i64 sz, i64 pos, i64 opt) { return bsd6(SYS_fgetxattr, fd, n, b, sz, pos, opt); }
static __attribute__((unused)) sysret fsx(i64 fd, const char *n, const void *b, i64 sz, i64 pos, i64 opt) { return bsd6(SYS_fsetxattr, fd, n, b, sz, pos, opt); }
static __attribute__((unused)) int user_names(const char *list, i64 len, char *out) {
    /* the NUL-separated names of a list without com.apple.provenance, joined by | */
    int n = 0;
    for (i64 i = 0; i < len;) {
        const char *nm = list + i;
        i64 l = (i64)str_len(nm) + 1;
        if (!(str_len(nm) >= 20 && mem_eq(nm, "com.apple.provenance", 20))) {
            if (n) out[n++] = 0x7c;
            for (i64 j = 0; j < l - 1; j++) out[n++] = nm[j];
        }
        i += l;
    }
    out[n] = 0;
    return n;
}
/* getxattr, BSD 234 (path, name, value, size, position, options): returns the length of the
 * attribute and copies it into the buffer. A NULL buffer with size 0 only asks for the length;
 * a buffer that is too small is ERANGE (34); an empty value is 0 bytes; a missing attribute is
 * ENOATTR (93); names are case sensitive. */
static void unused_dummy(void) {}
int test_main(void) {
    unused_dummy();
    bsd1(SYS_close, bsd3(SYS_open, "file", O_CREAT | O_RDWR, 0644).ret);
    char buf[64];
    sysret r = gx("file", "user.none", buf, sizeof buf, 0, 0);
    check_err("a missing attribute is ENOATTR", r, ENOATTR);
    r = sx("file", "user.a", "hello", 5, 0, 0);
    check_ok("setxattr", r);
    memset(buf, 0, sizeof buf);
    r = gx("file", "user.a", buf, sizeof buf, 0, 0);
    check("the length is 5", r.err == 0 && r.ret == 5, r.err ? r.err : r.ret);
    check("the value is copied", mem_eq(buf, "hello", 5), 0);
    check("nothing is copied past the length", buf[5] == 0, buf[5]);
    r = gx("file", "user.a", 0, 0, 0, 0);
    check("a NULL buffer asks for the length", r.err == 0 && r.ret == 5, r.err ? r.err : r.ret);
    r = gx("file", "user.a", buf, 5, 0, 0);
    check("a buffer of exactly the length works", r.err == 0 && r.ret == 5, r.err ? r.err : r.ret);
    r = gx("file", "user.a", buf, 4, 0, 0);
    check_err("a buffer one byte short is ERANGE", r, ERANGE);
    r = gx("file", "user.a", buf, 0, 0, 0);
    check_err("size 0 with a buffer is ERANGE (only a NULL buffer asks for the length)", r, ERANGE);
    r = gx("file", "USER.A", buf, sizeof buf, 0, 0);
    check_err("names are case sensitive", r, ENOATTR);
    r = sx("file", "user.empty", "", 0, 0, 0);
    check_ok("an empty value can be set", r);
    r = gx("file", "user.empty", buf, sizeof buf, 0, 0);
    check("and reads back with length 0", r.err == 0 && r.ret == 0, r.err ? r.err : r.ret);
    char bin[6] = {0, 1, 0, 2, 0, 3};
    sx("file", "user.bin", bin, 6, 0, 0);
    memset(buf, 9, sizeof buf);
    r = gx("file", "user.bin", buf, sizeof buf, 0, 0);
    check("a value with NUL bytes keeps them", r.err == 0 && r.ret == 6 && mem_eq(buf, bin, 6), r.err ? r.err : r.ret);
    r = gx("file", "user.a", buf, sizeof buf, 7, 0);
    check_err("a position other than 0 is EINVAL for an ordinary attribute", r, EINVAL);
    r = gx("file", "", buf, sizeof buf, 0, 0);
    check_err("an empty name is EINVAL", r, EINVAL);
    return finish();
}
