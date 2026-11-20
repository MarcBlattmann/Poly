#include "poly_mtest.h"
#define SYS_rename 128
#define SYS_renameat 465
#define SYS_renameatx_np 488
#define SYS_link 9
#define SYS_ftruncate 201
#define SYS_chmod 15
#define SYS_getattrlist 220
#define SYS_setattrlist 221
#define SYS_fgetattrlist 228
#define SYS_getattrlistbulk 461
#define SYS_clonefileat 462
#define SYS_fclonefileat 517
#define SYS_getxattr 234
#define SYS_setxattr 236
#define O_SYMLINK 0x200000
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
/* renameat, BSD 465: the same refusals as rename, through descriptors. A directory into itself or its own
 * subdirectory is EINVAL; a directory over a non-empty one ENOTEMPTY (66); over a file ENOTDIR (20); a file
 * over a directory EISDIR (21); a destination directory without write permission EACCES; an unreadable
 * path EFAULT; a descriptor of a directory that was removed afterwards fails the lookup with ENOENT. */
int test_main(void) {
    bsd2(SYS_mkdir, "top", 0755);
    bsd2(SYS_mkdir, "top/sub", 0755);
    int t = bsd3(SYS_open, "top", O_RDONLY, 0).ret;
    int cwd = AT_FDCWD;
    check_err("a directory into itself is EINVAL", bsd4(SYS_renameat, cwd, "top", t, "x"), EINVAL);
    check_err("into its own subdirectory is EINVAL", bsd4(SYS_renameat, cwd, "top", t, "sub/x"), EINVAL);
    bsd2(SYS_mkdir, "full", 0755);
    mkfile("full/x", "x");
    bsd2(SYS_mkdir, "d3", 0755);
    check_err("a directory over a non-empty one is ENOTEMPTY", bsd4(SYS_renameat, cwd, "d3", cwd, "full"), ENOTEMPTY);
    mkfile("file", "f");
    check_err("a directory over a file is ENOTDIR", bsd4(SYS_renameat, cwd, "d3", cwd, "file"), ENOTDIR);
    check_err("a file over a directory is EISDIR", bsd4(SYS_renameat, cwd, "file", cwd, "d3"), EISDIR);
    bsd2(SYS_mkdir, "ro", 0555);
    int ro = bsd3(SYS_open, "ro", O_RDONLY, 0).ret;
    check_err("a destination directory without write permission is EACCES", bsd4(SYS_renameat, cwd, "file", ro, "x"), EACCES);
    check_err("an unreadable source is EFAULT", bsd4(SYS_renameat, cwd, (char *)8, cwd, "x"), EFAULT);
    check_err("an unreadable destination is EFAULT", bsd4(SYS_renameat, cwd, "file", cwd, (char *)8), EFAULT);
    bsd2(SYS_mkdir, "gone", 0755);
    int g = bsd3(SYS_open, "gone", O_RDONLY, 0).ret;
    bsd1(SYS_rmdir, "gone");
    value("a name inside a removed directory (source): errno", bsd4(SYS_renameat, g, "x", cwd, "y").err);
    value("a destination inside a removed directory: errno", bsd4(SYS_renameat, cwd, "file", g, "y").err);
    check("the file is untouched", exists("file"), 0);
    return finish();
}
