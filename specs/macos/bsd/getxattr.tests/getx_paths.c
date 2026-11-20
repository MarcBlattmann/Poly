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
/* getxattr, BSD 234: path errors and links. A missing path is ENOENT, a path through a file
 * ENOTDIR, an unreadable pointer EFAULT; a descriptor number is not a path (EFAULT). A symbolic
 * link is followed unless XATTR_NOFOLLOW (1) is given, in which case the link's own attributes
 * are read; a loop of links is ELOOP (62) when followed. A file without read permission
 * refuses getxattr with EACCES (13). Attributes of directories work like those of files. */
int test_main(void) {
    char buf[32];
    sysret r = gx("nofile", "user.a", buf, sizeof buf, 0, 0);
    check_err("a missing file is ENOENT", r, ENOENT);
    bsd1(SYS_close, bsd3(SYS_open, "file", O_CREAT | O_RDWR, 0644).ret);
    sx("file", "user.a", "target", 6, 0, 0);
    r = gx("file/x", "user.a", buf, sizeof buf, 0, 0);
    check_err("a path through a file is ENOTDIR", r, ENOTDIR);
    r = gx("", "user.a", buf, sizeof buf, 0, 0);
    check_err("an empty path is ENOENT", r, ENOENT);
    r = gx("file", (char *)8, buf, sizeof buf, 0, 0);
    check_err("an unreadable name pointer is EFAULT", r, EFAULT);
    r = gx((char *)8, "user.a", buf, sizeof buf, 0, 0);
    check_err("an unreadable path pointer is EFAULT", r, EFAULT);
    r = gx("file", "user.a", (char *)8, 6, 0, 0);
    check_err("an unwritable buffer is EFAULT", r, EFAULT);
    bsd2(SYS_symlink, "file", "link");
    r = gx("link", "user.a", buf, sizeof buf, 0, 0);
    check("through a link: the target's attribute", r.err == 0 && r.ret == 6, r.err ? r.err : r.ret);
    r = gx("link", "user.a", buf, sizeof buf, 0, XATTR_NOFOLLOW);
    check_err("with XATTR_NOFOLLOW the link itself has none: ENOATTR", r, ENOATTR);
    r = sx("link", "user.own", "L", 1, 0, XATTR_NOFOLLOW);
    check_ok("an attribute can be set on the link itself", r);
    r = gx("link", "user.own", buf, sizeof buf, 0, XATTR_NOFOLLOW);
    check("and read from it", r.err == 0 && r.ret == 1 && buf[0] == 76, r.err ? r.err : r.ret);
    r = gx("link", "user.own", buf, sizeof buf, 0, 0);
    check_err("the target does not have the link's attribute", r, ENOATTR);
    bsd2(SYS_symlink, "loop2", "loop1");
    bsd2(SYS_symlink, "loop1", "loop2");
    r = gx("loop1", "user.a", buf, sizeof buf, 0, 0);
    check_err("a loop of links is ELOOP", r, ELOOP);
    r = gx("loop1", "user.a", buf, sizeof buf, 0, XATTR_NOFOLLOW);
    check_err("with XATTR_NOFOLLOW the loop is not followed: ENOATTR", r, ENOATTR);
    bsd2(SYS_symlink, "nowhere", "dangling");
    r = gx("dangling", "user.a", buf, sizeof buf, 0, 0);
    check_err("a dangling link is ENOENT", r, ENOENT);
    bsd2(SYS_mkdir, "dir", 0755);
    sx("dir", "user.d", "dd", 2, 0, 0);
    r = gx("dir", "user.d", buf, sizeof buf, 0, 0);
    check("a directory has attributes too", r.err == 0 && r.ret == 2, r.err ? r.err : r.ret);
    bsd1(SYS_close, bsd3(SYS_open, "locked", O_CREAT | O_RDWR, 0).ret);
    r = gx("locked", "user.a", buf, sizeof buf, 0, 0);
    check_err("a file with mode 0 refuses getxattr: EACCES", r, EACCES);
    r = gx("file", "user.a", buf, sizeof buf, 0, 0x10000);
    check("an option bit nobody defined is ignored", r.err == 0 && r.ret == 6, r.err ? r.err : r.ret);
    return finish();
}
