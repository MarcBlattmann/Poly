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
/* fsetxattr, BSD 237 (fd, name, value, size, position, options): XATTR_CREATE (2) is EEXIST for an
 * existing attribute, XATTR_REPLACE (4) ENOATTR for a missing one, both together EINVAL; a plain
 * set creates or replaces; it works through a read-only descriptor; bad descriptor EBADF; position
 * other than 0 EINVAL; options 8 and 0x10 EINVAL, undefined bits ignored; size 0 stores an empty
 * attribute. */
int test_main(void) {
    int fd = bsd3(SYS_open, "file", O_CREAT | O_RDWR, 0644).ret;
    char buf[64];
    check_err("REPLACE on a missing attribute is ENOATTR", fsx(fd, "user.a", "v", 1, 0, XATTR_REPLACE), ENOATTR);
    sysret r = fsx(fd, "user.a", "one", 3, 0, XATTR_CREATE);
    check("CREATE works and returns 0", r.err == 0 && r.ret == 0, r.err);
    check_err("CREATE on an existing attribute is EEXIST", fsx(fd, "user.a", "two", 3, 0, XATTR_CREATE), EEXIST);
    check_ok("REPLACE on an existing one", fsx(fd, "user.a", "longer", 6, 0, XATTR_REPLACE));
    r = gx("file", "user.a", buf, sizeof buf, 0, 0);
    check("path-based getxattr sees the new value", r.ret == 6 && mem_eq(buf, "longer", 6), r.err ? r.err : r.ret);
    check_err("CREATE|REPLACE is EINVAL", fsx(fd, "user.a", "x", 1, 0, 6), EINVAL);
    check_err("option 8 is EINVAL", fsx(fd, "user.a", "x", 1, 0, 8), EINVAL);
    check_err("option 0x10 is EINVAL", fsx(fd, "user.a", "x", 1, 0, 0x10), EINVAL);
    check_ok("an undefined option bit is ignored", fsx(fd, "user.a", "x", 1, 0, 0x100));
    check_err("a position other than 0 is EINVAL", fsx(fd, "user.a", "x", 1, 3, 0), EINVAL);
    check_err("a bad descriptor is EBADF", fsx(99, "user.a", "x", 1, 0, 0), EBADF);
    check_err("an empty name is EINVAL", fsx(fd, "", "x", 1, 0, 0), EINVAL);
    check_err("an unreadable value is EFAULT", fsx(fd, "user.a", (char *)8, 4, 0, 0), EFAULT);
    int ro = bsd3(SYS_open, "file", O_RDONLY, 0).ret;
    check_ok("a read-only descriptor can set attributes", fsx(ro, "user.viaro", "r", 1, 0, 0));
    r = gx("file", "user.viaro", buf, sizeof buf, 0, 0);
    check("and it is there", r.err == 0 && r.ret == 1, r.err ? r.err : r.ret);
    check_ok("size 0 stores an empty attribute", fsx(fd, "user.empty", "", 0, 0, 0));
    check("which has length 0", gx("file", "user.empty", buf, sizeof buf, 0, 0).ret == 0, 0);
    return finish();
}
