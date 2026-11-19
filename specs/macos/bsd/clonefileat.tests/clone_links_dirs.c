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
/* clonefileat, BSD 462: symbolic links and directories. A link as the source is followed (the copy
 * is a regular file with the target's content) unless CLONE_NOFOLLOW (1), which copies the link
 * itself; a dangling link followed is ENOENT; a directory is cloned with everything in it
 * (files, subdirectories, links, with their content) and the copy is independent; a destination
 * directory that exists is EEXIST. Flag bit 8 is accepted, 0x10000 is EINVAL (22). */
int test_main(void) {
    mkfile("src", "content");
    bsd2(SYS_symlink, "src", "link");
    sysret r = bsd6(SYS_clonefileat, AT_FDCWD, "link", AT_FDCWD, "c1", 0, 0);
    check_ok("clone through a link", r);
    struct poly_stat64 st;
    bsd2(SYS_lstat64, "c1", &st);
    check("the copy is a regular file", (st.st_mode & S_IFMT) == S_IFREG && content_is("c1", "content"), st.st_mode);
    r = bsd6(SYS_clonefileat, AT_FDCWD, "link", AT_FDCWD, "c2", CLONE_NOFOLLOW, 0);
    check_ok("clone with CLONE_NOFOLLOW", r);
    bsd2(SYS_lstat64, "c2", &st);
    check("the copy is a symbolic link", (st.st_mode & S_IFMT) == S_IFLNK, st.st_mode);
    char buf[16];
    memset(buf, 0, sizeof buf);
    bsd3(SYS_readlink, "c2", buf, sizeof buf);
    check("with the same target \"src\"", str_eq(buf, "src"), 0);
    bsd2(SYS_symlink, "nowhere", "dangling");
    check_err("a dangling link followed is ENOENT", bsd6(SYS_clonefileat, AT_FDCWD, "dangling", AT_FDCWD, "c3", 0, 0), ENOENT);
    check_ok("not followed, a dangling link can be cloned", bsd6(SYS_clonefileat, AT_FDCWD, "dangling", AT_FDCWD, "c3", CLONE_NOFOLLOW, 0));
    bsd2(SYS_mkdir, "tree", 0755);
    bsd2(SYS_mkdir, "tree/sub", 0750);
    mkfile("tree/a", "AAA");
    mkfile("tree/sub/b", "BBBBB");
    bsd2(SYS_symlink, "a", "tree/sub/ln");
    r = bsd6(SYS_clonefileat, AT_FDCWD, "tree", AT_FDCWD, "copy", 0, 0);
    check_ok("clone a directory", r);
    check("its files and subdirectories are there with content", content_is("copy/a", "AAA") && content_is("copy/sub/b", "BBBBB"), 0);
    bsd2(SYS_lstat64, "copy/sub/ln", &st);
    check("a link inside is a link", (st.st_mode & S_IFMT) == S_IFLNK, st.st_mode);
    bsd2(SYS_stat64, "copy/sub", &st);
    check("the subdirectory's mode is kept (0750)", (st.st_mode & 07777) == 0750, st.st_mode & 07777);
    mkfile("copy/new", "N");
    check("the copy is independent: a new file is not in the original", !exists("tree/new"), 0);
    bsd1(SYS_unlink, "tree/a");
    check("and removing from the original keeps the copy", content_is("copy/a", "AAA"), 0);
    check_err("a destination directory that exists is EEXIST", bsd6(SYS_clonefileat, AT_FDCWD, "tree", AT_FDCWD, "copy", 0, 0), EEXIST);
    check_ok("flag bit 8 is accepted", bsd6(SYS_clonefileat, AT_FDCWD, "src", AT_FDCWD, "f8", 8, 0));
    check_err("flag bit 0x10000 is EINVAL", bsd6(SYS_clonefileat, AT_FDCWD, "src", AT_FDCWD, "f9", 0x10000, 0), EINVAL);
    return finish();
}
