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
/* flistxattr, BSD 241 (fd, namebuf, size, options): like listxattr for an open descriptor: the
 * names each ended by a NUL, the byte count returned; a NULL buffer asks for it; one byte short
 * is ERANGE (34); a bad descriptor EBADF (9); options other than 0 are EINVAL for the
 * descriptor form. Files can carry a "com.apple.provenance" attribute of their own, which the
 * checks leave out. */
int test_main(void) {
    int fd = bsd3(SYS_open, "file", O_CREAT | O_RDWR, 0644).ret;
    char raw[1024], names[1024];
    sysret r = bsd4(SYS_flistxattr, fd, raw, sizeof raw, 0);
    check_ok("a new file", r);
    user_names(raw, r.ret, names);
    check("has no attributes of its own", names[0] == 0, names[0]);
    fsx(fd, "user.alpha", "1", 1, 0, 0);
    fsx(fd, "user.beta", "", 0, 0, 0);
    r = bsd4(SYS_flistxattr, fd, raw, sizeof raw, 0);
    i64 n = user_names(raw, r.ret, names);
    check("two names: user.alpha|user.beta in byte order", str_eq(names, "user.alpha|user.beta"), n);
    i64 total = bsd4(SYS_flistxattr, fd, 0, 0, 0).ret;
    check("a NULL buffer gives the byte count of the full list", total == r.ret && total >= 11 + 10, total);
    check("a buffer of exactly that size works", bsd4(SYS_flistxattr, fd, raw, total, 0).ret == total, 0);
    check_err("one byte short is ERANGE", bsd4(SYS_flistxattr, fd, raw, total - 1, 0), ERANGE);
    check_err("a bad descriptor is EBADF", bsd4(SYS_flistxattr, 99, raw, sizeof raw, 0), EBADF);
    check_err("option 1 is EINVAL for the descriptor form", bsd4(SYS_flistxattr, fd, raw, sizeof raw, XATTR_NOFOLLOW), EINVAL);
    check_err("an unwritable buffer is EFAULT", bsd4(SYS_flistxattr, fd, (char *)8, total, 0), EFAULT);
    bsd3(SYS_removexattr, "file", "user.alpha", 0);
    r = bsd4(SYS_flistxattr, fd, raw, sizeof raw, 0);
    user_names(raw, r.ret, names);
    check("a removal through the path shows in the list", str_eq(names, "user.beta"), 0);
    int ro = bsd3(SYS_open, "file", O_RDONLY, 0).ret;
    r = bsd4(SYS_flistxattr, ro, raw, sizeof raw, 0);
    user_names(raw, r.ret, names);
    check("a read-only descriptor lists too", r.err == 0 && str_eq(names, "user.beta"), r.err);
    return finish();
}
