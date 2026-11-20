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
/* setxattr, BSD 236: what can carry attributes and what is refused. Regular files, directories and
 * symbolic links (with XATTR_NOFOLLOW) can. A missing file is ENOENT; a file with mode 0 or 0444 is
 * EACCES (13); an unreadable pointer EFAULT; attributes stay with the file across rename, hard
 * links, truncation and the removal of one name; the resource fork can be written at a position
 * (a gap reads as zeros) and FinderInfo takes exactly 32 bytes (ERANGE 34 otherwise). */
int test_main(void) {
    char buf[64];
    sysret r = sx("nofile", "user.a", "x", 1, 0, 0);
    check_err("a missing file is ENOENT", r, ENOENT);
    bsd1(SYS_close, bsd3(SYS_open, "ro", O_CREAT | O_RDWR, 0444).ret);
    r = sx("ro", "user.a", "x", 1, 0, 0);
    check_err("a file with mode 0444 is EACCES, even for its owner", r, EACCES);
    r = sx("file", "user.a", (char *)8, 4, 0, 0);
    check_err("an unreadable value pointer is EFAULT (the file is missing too: ENOENT comes first)", r, ENOENT);
    bsd1(SYS_close, bsd3(SYS_open, "file", O_CREAT | O_RDWR, 0644).ret);
    r = sx("file", "user.a", (char *)8, 4, 0, 0);
    check_err("an unreadable value pointer is EFAULT", r, EFAULT);
    bsd2(SYS_mkdir, "dir", 0755);
    check_ok("a directory", sx("dir", "user.a", "d", 1, 0, 0));
    sx("file", "user.keep", "kept", 4, 0, 0);
    bsd2(SYS_rename, "file", "renamed");
    r = gx("renamed", "user.keep", buf, sizeof buf, 0, 0);
    check("an attribute survives rename", r.err == 0 && r.ret == 4, r.err ? r.err : r.ret);
    bsd2(SYS_link, "renamed", "hard");
    r = gx("hard", "user.keep", buf, sizeof buf, 0, 0);
    check("and is seen through a hard link", r.err == 0 && r.ret == 4, r.err ? r.err : r.ret);
    bsd2(SYS_ftruncate, bsd3(SYS_open, "renamed", O_RDWR, 0).ret, 0);
    r = gx("renamed", "user.keep", buf, sizeof buf, 0, 0);
    check("survives truncation", r.err == 0 && r.ret == 4, r.err ? r.err : r.ret);
    bsd1(SYS_unlink, "renamed");
    r = gx("hard", "user.keep", buf, sizeof buf, 0, 0);
    check("survives the removal of the other name", r.err == 0 && r.ret == 4, r.err ? r.err : r.ret);
    sx("hard", "com.apple.ResourceFork", "0123456789", 10, 0, 0);
    r = sx("hard", "com.apple.ResourceFork", "AB", 2, 4, 0);
    check_ok("the resource fork can be written at a position", r);
    r = gx("hard", "com.apple.ResourceFork", buf, sizeof buf, 0, 0);
    check("the bytes at that position changed, the length did not", r.ret == 10 && mem_eq(buf, "0123AB6789", 10), r.err ? r.err : r.ret);
    r = sx("hard", "com.apple.ResourceFork", "Z", 1, 20, 0);
    check_ok("writing beyond the end", r);
    r = gx("hard", "com.apple.ResourceFork", buf, sizeof buf, 0, 0);
    check("makes it 21 bytes with a gap of zeros", r.ret == 21 && buf[10] == 0 && buf[19] == 0 && buf[20] == 90, r.err ? r.err : r.ret);
    char fi[33];
    memset(fi, 1, sizeof fi);
    check_err("FinderInfo of 31 bytes is ERANGE", sx("hard", "com.apple.FinderInfo", fi, 31, 0, 0), ERANGE);
    check_err("of 33 bytes too", sx("hard", "com.apple.FinderInfo", fi, 33, 0, 0), ERANGE);
    check_ok("of 32 bytes", sx("hard", "com.apple.FinderInfo", fi, 32, 0, 0));
    return finish();
}
