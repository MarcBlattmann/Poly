#include "poly_mtest.h"
#define SYS_rename 128
#define SYS_renameat 465
#define SYS_renameatx_np 488
#define SYS_link 9
#define SYS_ftruncate 201
#define SYS_getattrlist 220
#define SYS_setattrlist 221
#define SYS_fgetattrlist 228
#define SYS_getattrlistbulk 461
#define SYS_clonefileat 462
#define SYS_fclonefileat 517
#define SYS_getxattr 234
#define SYS_setxattr 236
#define RENAME_SWAP 2
#define RENAME_EXCL 4
#define CLONE_NOFOLLOW 1
#define ATTR_BIT_MAP_COUNT 5
#define ATTR_CMN_RETURNED_ATTRS 0x80000000u
#define ATTR_CMN_NAME 0x1u
#define ATTR_CMN_OBJTYPE 0x8u
#define ATTR_FILE_DATALENGTH 0x200u
#define VREG 1
#define VDIR 2
#define VLNK 5
struct attrlist_t { u16 bitmapcount; u16 reserved; u32 commonattr, volattr, dirattr, fileattr, forkattr; };
static __attribute__((unused)) void mkfile(const char *path, const char *text) {
    sysret f = bsd3(SYS_open, path, O_CREAT | O_TRUNC | O_RDWR, 0644);
    if (!f.err) { bsd3(SYS_write, f.ret, text, str_len(text)); bsd1(SYS_close, f.ret); }
}
static __attribute__((unused)) i64 size_of(const char *path) {
    struct poly_stat64 st;
    return bsd2(SYS_stat64, path, &st).err ? -1 : st.st_size;
}
static __attribute__((unused)) i64 ino_of(const char *path) {
    struct poly_stat64 st;
    return bsd2(SYS_stat64, path, &st).err ? -1 : (i64)st.st_ino;
}
static __attribute__((unused)) int exists(const char *path) {
    struct poly_stat64 st;
    return bsd2(SYS_lstat64, path, &st).err == 0;
}
static __attribute__((unused)) int content_is(const char *path, const char *text) {
    char buf[256];
    memset(buf, 0, sizeof buf);
    sysret f = bsd3(SYS_open, path, O_RDONLY, 0);
    if (f.err) return 0;
    sysret r = bsd3(SYS_read, f.ret, buf, sizeof buf - 1);
    bsd1(SYS_close, f.ret);
    return r.err == 0 && (size_t)r.ret == str_len(text) && mem_eq(buf, text, str_len(text));
}
/* setattrlist, BSD 221: refusals. Attributes that cannot be set (the name 0x1, the object type 0x8)
 * are EINVAL (22); a buffer smaller than the attributes need is EINVAL; a missing file is ENOENT; a
 * path through a file ENOTDIR; an unreadable path EFAULT; the options FSOPT_NOFOLLOW (1) applies the
 * change to a symbolic link itself instead of its target; without it a link's target changes; a
 * dangling link followed is ENOENT. */
#define ATTR_CMN_MODTIME 0x400u
int test_main(void) {
    mkfile("file", "x");
    char buf[64];
    memset(buf, 0, sizeof buf);
    struct attrlist_t nm = {ATTR_BIT_MAP_COUNT, 0, ATTR_CMN_NAME, 0, 0, 0, 0};
    check_err("the name cannot be set: EINVAL", bsd6(SYS_setattrlist, "file", &nm, buf, 16, 0, 0), EINVAL);
    struct attrlist_t ot = {ATTR_BIT_MAP_COUNT, 0, ATTR_CMN_OBJTYPE, 0, 0, 0, 0};
    check_err("the object type cannot be set: EINVAL", bsd6(SYS_setattrlist, "file", &ot, buf, 4, 0, 0), EINVAL);
    struct attrlist_t m = {ATTR_BIT_MAP_COUNT, 0, ATTR_CMN_MODTIME, 0, 0, 0, 0};
    struct poly_timespec ts = {1000000000, 0};
    check_err("a buffer too small for a timespec is EINVAL", bsd6(SYS_setattrlist, "file", &m, &ts, 8, 0, 0), EINVAL);
    check_err("a missing file is ENOENT", bsd6(SYS_setattrlist, "nofile", &m, &ts, sizeof ts, 0, 0), ENOENT);
    check_err("a path through a file is ENOTDIR", bsd6(SYS_setattrlist, "file/x", &m, &ts, sizeof ts, 0, 0), ENOTDIR);
    check_err("an unreadable path is EFAULT", bsd6(SYS_setattrlist, (char *)8, &m, &ts, sizeof ts, 0, 0), EFAULT);
    bsd2(SYS_symlink, "file", "link");
    struct poly_stat64 st;
    check_ok("through a link", bsd6(SYS_setattrlist, "link", &m, &ts, sizeof ts, 0, 0));
    bsd2(SYS_stat64, "file", &st);
    check("the target changed", st.st_mtimespec.tv_sec == 1000000000, st.st_mtimespec.tv_sec);
    struct poly_timespec t2 = {1500000000, 0};
    check_ok("with FSOPT_NOFOLLOW", bsd6(SYS_setattrlist, "link", &m, &t2, sizeof t2, 1, 0));
    bsd2(SYS_stat64, "file", &st);
    check("the target did not", st.st_mtimespec.tv_sec == 1000000000, st.st_mtimespec.tv_sec);
    bsd2(SYS_lstat64, "link", &st);
    check("the link itself did", st.st_mtimespec.tv_sec == 1500000000, st.st_mtimespec.tv_sec);
    bsd2(SYS_symlink, "nowhere", "dangling");
    check_err("a dangling link followed is ENOENT", bsd6(SYS_setattrlist, "dangling", &m, &ts, sizeof ts, 0, 0), ENOENT);
    check_ok("a dangling link with FSOPT_NOFOLLOW works", bsd6(SYS_setattrlist, "dangling", &m, &ts, sizeof ts, 1, 0));
    bsd2(SYS_mkdir, "dir", 0755);
    check_ok("a directory's modification time", bsd6(SYS_setattrlist, "dir", &m, &ts, sizeof ts, 0, 0));
    bsd2(SYS_stat64, "dir", &st);
    check("is changed", st.st_mtimespec.tv_sec == 1000000000, st.st_mtimespec.tv_sec);
    return finish();
}
