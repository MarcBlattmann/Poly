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
/* listxattr, BSD 240 (path, namebuf, size, options): stores the attribute names, each ended by a
 * NUL, and returns the number of bytes. A NULL buffer asks for the number; a buffer that is too
 * small is ERANGE (34). Files created by a process on this Mac can already carry a
 * "com.apple.provenance" attribute, which the checks leave out. */
static i64 list(const char *path, char *out, i64 opt) {
    char raw[1024];
    sysret r = bsd4(SYS_listxattr, path, raw, sizeof raw, opt);
    if (r.err) return -r.err;
    return user_names(raw, r.ret, out);
}
int test_main(void) {
    bsd1(SYS_close, bsd3(SYS_open, "file", O_CREAT | O_RDWR, 0644).ret);
    char names[1024];
    check("a new file has no attributes of its own", list("file", names, 0) == 0, list("file", names, 0));
    sx("file", "user.one", "1", 1, 0, 0);
    sx("file", "user.two", "22", 2, 0, 0);
    sx("file", "user.three", "", 0, 0, 0);
    i64 n = list("file", names, 0);
    check("three names, joined by |: user.one|user.two|user.three in some order", n == 8 + 8 + 10 + 2, n);
    check("user.one is among them", n > 0 && (str_len(names) >= 8), 0);
    int seen = 0;
    for (i64 i = 0; i + 8 <= n; i++) {
        if (mem_eq(names + i, "user.one", 8)) seen |= 1;
        if (mem_eq(names + i, "user.two", 8)) seen |= 2;
        if (i + 10 <= n && mem_eq(names + i, "user.three", 10)) seen |= 4;
    }
    check("each of the three is listed", seen == 7, seen);
    char raw[1024];
    sysret r = bsd4(SYS_listxattr, "file", 0, 0, 0);
    i64 total = r.ret;
    check("a NULL buffer: the byte count", r.err == 0 && total >= 8 + 8 + 10 + 3, r.err ? r.err : r.ret);
    r = bsd4(SYS_listxattr, "file", raw, total, 0);
    check("a buffer of exactly that size returns the same count", r.err == 0 && r.ret == total, r.err ? r.err : r.ret);
    check("the list ends with a NUL", raw[total - 1] == 0, raw[total - 1]);
    r = bsd4(SYS_listxattr, "file", raw, total - 1, 0);
    check_err("a buffer one byte short is ERANGE", r, ERANGE);
    r = bsd4(SYS_listxattr, "file", raw, 0, 0);
    check("size 0 with a buffer: the byte count", r.err == 0 && r.ret == total, r.err ? r.err : r.ret);
    r = bsd4(SYS_listxattr, "file", (char *)8, total, 0);
    check_err("an unwritable buffer is EFAULT", r, EFAULT);
    sx("file", "user.one", "1", 1, 0, 0);
    check("setting an existing name again adds nothing", list("file", names, 0) == n, 0);
    bsd3(SYS_removexattr, "file", "user.two", 0);
    check("a removed name is gone", list("file", names, 0) == 8 + 10 + 1, list("file", names, 0));
    return finish();
}
