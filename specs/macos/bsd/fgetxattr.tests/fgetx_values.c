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
/* fgetxattr, BSD 235 (fd, name, value, size, position, options): like getxattr on the file an
 * open descriptor refers to, also one opened read-only. Returns the length; a NULL buffer asks
 * for it; too small a buffer is ERANGE (34); a missing attribute ENOATTR (93); names are case
 * sensitive; a bad descriptor is EBADF (9); a position other than 0 is EINVAL except for the
 * resource fork; options 1 (XATTR_NOFOLLOW) is EINVAL here. */
int test_main(void) {
    int fd = bsd3(SYS_open, "file", O_CREAT | O_RDWR, 0644).ret;
    char buf[64];
    sx("file", "user.a", "hello", 5, 0, 0);
    memset(buf, 0, sizeof buf);
    sysret r = fgx(fd, "user.a", buf, sizeof buf, 0, 0);
    check("the length is 5", r.err == 0 && r.ret == 5 && mem_eq(buf, "hello", 5), r.err ? r.err : r.ret);
    check("a NULL buffer asks for the length", fgx(fd, "user.a", 0, 0, 0, 0).ret == 5, 0);
    check_err("one byte short is ERANGE", fgx(fd, "user.a", buf, 4, 0, 0), ERANGE);
    check_err("a missing attribute is ENOATTR", fgx(fd, "user.none", buf, sizeof buf, 0, 0), ENOATTR);
    check_err("names are case sensitive", fgx(fd, "USER.A", buf, sizeof buf, 0, 0), ENOATTR);
    check_err("an empty name is EINVAL", fgx(fd, "", buf, sizeof buf, 0, 0), EINVAL);
    check_err("a bad descriptor is EBADF", fgx(99, "user.a", buf, sizeof buf, 0, 0), EBADF);
    check_err("a closed descriptor is EBADF", fgx(fd + 40, "user.a", buf, sizeof buf, 0, 0), EBADF);
    check_err("a position other than 0 is EINVAL", fgx(fd, "user.a", buf, sizeof buf, 3, 0), EINVAL);
    check_err("option 1 is EINVAL for the descriptor form", fgx(fd, "user.a", buf, sizeof buf, 0, XATTR_NOFOLLOW), EINVAL);
    check_err("an unwritable buffer is EFAULT", fgx(fd, "user.a", (char *)8, 5, 0, 0), EFAULT);
    int ro = bsd3(SYS_open, "file", O_RDONLY, 0).ret;
    check("through a read-only descriptor", fgx(ro, "user.a", buf, sizeof buf, 0, 0).ret == 5, 0);
    int dir = bsd3(SYS_open, ".", O_RDONLY, 0).ret;
    sx(".", "user.dir", "dd", 2, 0, 0);
    check("a directory descriptor", fgx(dir, "user.dir", buf, sizeof buf, 0, 0).ret == 2, 0);
    sx("file", "com.apple.ResourceFork", "0123456789", 10, 0, 0);
    r = fgx(fd, "com.apple.ResourceFork", buf, 4, 3, 0);
    check("the resource fork can be read at a position", r.err == 0 && r.ret == 4 && mem_eq(buf, "3456", 4), r.err ? r.err : r.ret);
    return finish();
}
