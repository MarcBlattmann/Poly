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
/* removexattr, BSD 238 (path, name, options): removes one attribute and returns 0; the same
 * name again, or one that was never set, is ENOATTR (93); the other attributes stay; a missing
 * file is ENOENT; an empty name EINVAL; an unreadable name EFAULT; with XATTR_NOFOLLOW (1) a link's
 * own attributes are removed, not the target's. */
int test_main(void) {
    bsd1(SYS_close, bsd3(SYS_open, "file", O_CREAT | O_RDWR, 0644).ret);
    char buf[16];
    sx("file", "user.a", "a", 1, 0, 0);
    sx("file", "user.b", "b", 1, 0, 0);
    sysret r = bsd3(SYS_removexattr, "file", "user.a", 0);
    check("remove returns 0", r.err == 0 && r.ret == 0, r.err);
    check_err("the attribute is gone", gx("file", "user.a", buf, sizeof buf, 0, 0), ENOATTR);
    check("the other one is still there", gx("file", "user.b", buf, sizeof buf, 0, 0).ret == 1, 0);
    check_err("removing it again is ENOATTR", bsd3(SYS_removexattr, "file", "user.a", 0), ENOATTR);
    check_err("removing one never set is ENOATTR", bsd3(SYS_removexattr, "file", "user.never", 0), ENOATTR);
    check_err("a missing file is ENOENT", bsd3(SYS_removexattr, "nofile", "user.a", 0), ENOENT);
    check_err("an empty name is EINVAL", bsd3(SYS_removexattr, "file", "", 0), EINVAL);
    check_err("an unreadable name is EFAULT", bsd3(SYS_removexattr, "file", (char *)8, 0), EFAULT);
    check_err("an unreadable path is EFAULT", bsd3(SYS_removexattr, (char *)8, "user.a", 0), EFAULT);
    check_err("a path through a file is ENOTDIR", bsd3(SYS_removexattr, "file/x", "user.a", 0), ENOTDIR);
    sx("file", "user.a", "again", 5, 0, 0);
    check("a removed name can be set again", gx("file", "user.a", buf, sizeof buf, 0, 0).ret == 5, 0);
    bsd2(SYS_symlink, "file", "link");
    sx("link", "user.own", "o", 1, 0, XATTR_NOFOLLOW);
    check_err("through the link the target has no 'user.own'", bsd3(SYS_removexattr, "link", "user.own", 0), ENOATTR);
    check_ok("with XATTR_NOFOLLOW the link's own is removed", bsd3(SYS_removexattr, "link", "user.own", XATTR_NOFOLLOW));
    check("the target's attribute through the link is untouched", bsd3(SYS_removexattr, "link", "user.a", 0).err == 0, 0);
    check_err("and now gone from the target", gx("file", "user.a", buf, sizeof buf, 0, 0), ENOATTR);
    bsd2(SYS_mkdir, "dir", 0755);
    sx("dir", "user.d", "d", 1, 0, 0);
    check_ok("a directory's attribute", bsd3(SYS_removexattr, "dir", "user.d", 0));
    return finish();
}
