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
/* fremovexattr, BSD 239 (fd, name, options): removes one attribute through a descriptor and
 * returns 0; again, or a name never set, is ENOATTR (93); a bad descriptor EBADF (9); an empty name
 * EINVAL; option 1 EINVAL for the descriptor form; other attributes stay; it works through a
 * read-only descriptor; the effect is visible by path. */
int test_main(void) {
    int fd = bsd3(SYS_open, "file", O_CREAT | O_RDWR, 0644).ret;
    char buf[16];
    fsx(fd, "user.a", "a", 1, 0, 0);
    fsx(fd, "user.b", "b", 1, 0, 0);
    sysret r = bsd3(SYS_fremovexattr, fd, "user.a", 0);
    check("remove returns 0", r.err == 0 && r.ret == 0, r.err);
    check_err("the attribute is gone (by path)", gx("file", "user.a", buf, sizeof buf, 0, 0), ENOATTR);
    check("the other stays", gx("file", "user.b", buf, sizeof buf, 0, 0).ret == 1, 0);
    check_err("removing it again is ENOATTR", bsd3(SYS_fremovexattr, fd, "user.a", 0), ENOATTR);
    check_err("one never set is ENOATTR", bsd3(SYS_fremovexattr, fd, "user.never", 0), ENOATTR);
    check_err("a bad descriptor is EBADF", bsd3(SYS_fremovexattr, 99, "user.b", 0), EBADF);
    check_err("an empty name is EINVAL", bsd3(SYS_fremovexattr, fd, "", 0), EINVAL);
    check_err("option 1 is EINVAL for the descriptor form", bsd3(SYS_fremovexattr, fd, "user.b", XATTR_NOFOLLOW), EINVAL);
    check_err("an unreadable name is EFAULT", bsd3(SYS_fremovexattr, fd, (char *)8, 0), EFAULT);
    int ro = bsd3(SYS_open, "file", O_RDONLY, 0).ret;
    check_ok("a read-only descriptor can remove", bsd3(SYS_fremovexattr, ro, "user.b", 0));
    check_err("and it is gone", gx("file", "user.b", buf, sizeof buf, 0, 0), ENOATTR);
    fsx(fd, "user.a", "again", 5, 0, 0);
    check("a removed name can be set again", fgx(fd, "user.a", buf, sizeof buf, 0, 0).ret == 5, 0);
    return finish();
}
