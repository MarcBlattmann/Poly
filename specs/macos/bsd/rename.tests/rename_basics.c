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
/* rename, BSD 128 (from, to): renames a file or directory and returns 0. An existing file at the
 * destination is replaced atomically; a directory may replace an empty directory (ENOTEMPTY 66 when
 * it is not empty); a file over a directory is EISDIR (21), a directory over a file ENOTDIR (20);
 * renaming a name to itself, or to another hard link of the same file, does nothing and returns 0 (both
 * names stay); descriptors opened before follow the file; the inode and times stay the same;
 * a directory moves with its content. */
int test_main(void) {
    mkfile("a", "AAA");
    mkfile("b", "BB");
    i64 ino = ino_of("a");
    sysret r = bsd2(SYS_rename, "a", "b");
    check("rename over an existing file: 0", r.err == 0 && r.ret == 0, r.err);
    check("the source name is gone", !exists("a"), 0);
    check("the destination has the source's content and inode", content_is("b", "AAA") && ino_of("b") == ino, 0);
    r = bsd2(SYS_rename, "b", "b");
    check("a name to itself: 0", r.err == 0, r.err);
    check("and the file is still there", content_is("b", "AAA"), 0);
    bsd2(SYS_link, "b", "hl");
    r = bsd2(SYS_rename, "b", "hl");
    check("onto its own hard link: 0", r.err == 0, r.err);
    check("both names stay", exists("b") && exists("hl"), 0);
    int fd = bsd3(SYS_open, "b", O_RDWR, 0).ret;
    bsd2(SYS_rename, "b", "moved");
    bsd3(SYS_write, fd, "Z", 1);
    check("an open descriptor follows the rename", content_is("moved", "ZAA"), 0);
    bsd2(SYS_mkdir, "d1", 0755);
    bsd2(SYS_mkdir, "d2", 0755);
    check_ok("a directory over an empty directory", bsd2(SYS_rename, "d1", "d2"));
    check("the source is gone", !exists("d1"), 0);
    bsd2(SYS_mkdir, "full", 0755);
    mkfile("full/x", "x");
    bsd2(SYS_mkdir, "d3", 0755);
    check_err("a directory over a non-empty one: ENOTEMPTY", bsd2(SYS_rename, "d3", "full"), ENOTEMPTY);
    check("nothing changed", exists("d3") && exists("full/x"), 0);
    check_err("a directory over a file: ENOTDIR", bsd2(SYS_rename, "d3", "moved"), ENOTDIR);
    check_err("a file over a directory: EISDIR", bsd2(SYS_rename, "moved", "d3"), EISDIR);
    check_ok("a directory with content moves", bsd2(SYS_rename, "full", "full2"));
    check("its content moved with it", content_is("full2/x", "x") && !exists("full"), 0);
    bsd2(SYS_symlink, "full2", "sl");
    check_ok("a symbolic link is renamed itself", bsd2(SYS_rename, "sl", "sl2"));
    check("it is still a link to full2", exists("sl2") && !exists("sl"), 0);
    struct poly_stat64 st;
    bsd2(SYS_lstat64, "sl2", &st);
    check("lstat says link", (st.st_mode & S_IFMT) == S_IFLNK, st.st_mode);
    return finish();
}
