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
/* listxattr, BSD 240: paths and links. A missing path is ENOENT, a path through a file ENOTDIR,
 * an unreadable path EFAULT, a file with mode 0 EACCES. A symbolic link is followed unless
 * XATTR_NOFOLLOW (1): then the link's own attributes are listed; a directory can be listed. */
static i64 count_user(const char *path, i64 opt) {
    char raw[1024], names[1024];
    sysret r = bsd4(SYS_listxattr, path, raw, sizeof raw, opt);
    if (r.err) return -r.err;
    return user_names(raw, r.ret, names);
}
int test_main(void) {
    char raw[256];
    check_err("a missing path is ENOENT", bsd4(SYS_listxattr, "nofile", raw, sizeof raw, 0), ENOENT);
    bsd1(SYS_close, bsd3(SYS_open, "file", O_CREAT | O_RDWR, 0644).ret);
    check_err("a path through a file is ENOTDIR", bsd4(SYS_listxattr, "file/x", raw, sizeof raw, 0), ENOTDIR);
    check_err("an unreadable path is EFAULT", bsd4(SYS_listxattr, (char *)8, raw, sizeof raw, 0), EFAULT);
    bsd1(SYS_close, bsd3(SYS_open, "locked", O_CREAT | O_RDWR, 0).ret);
    check_err("a file with mode 0 is EACCES", bsd4(SYS_listxattr, "locked", raw, sizeof raw, 0), EACCES);
    sx("file", "user.target", "t", 1, 0, 0);
    bsd2(SYS_symlink, "file", "link");
    check("through a link: the target's names (user.target, 11 bytes)", count_user("link", 0) == 11, count_user("link", 0));
    check("with XATTR_NOFOLLOW: the link's own, none yet", count_user("link", XATTR_NOFOLLOW) == 0, count_user("link", XATTR_NOFOLLOW));
    sx("link", "user.mine", "m", 1, 0, XATTR_NOFOLLOW);
    check("after setting one on the link (user.mine, 10 bytes)", count_user("link", XATTR_NOFOLLOW) == 9, count_user("link", XATTR_NOFOLLOW));
    check("the target still has only its own", count_user("file", 0) == 11, count_user("file", 0));
    bsd2(SYS_mkdir, "dir", 0755);
    sx("dir", "user.d", "d", 1, 0, 0);
    check("a directory (user.d, 7 bytes)", count_user("dir", 0) == 6, count_user("dir", 0));
    bsd2(SYS_symlink, "nowhere", "dangling");
    check_err("a dangling link followed is ENOENT", bsd4(SYS_listxattr, "dangling", raw, sizeof raw, 0), ENOENT);
    check("not followed it can be listed", count_user("dangling", XATTR_NOFOLLOW) == 0, count_user("dangling", XATTR_NOFOLLOW));
    return finish();
}
