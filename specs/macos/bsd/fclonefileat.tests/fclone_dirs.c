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
/* fclonefileat, BSD 517: directories and links through a descriptor. A descriptor of a directory
 * clones the whole tree (files, subdirectories with their modes, content); an O_SYMLINK (0x200000)
 * descriptor of a link clones the link itself (the copy is a link with the same target); a
 * descriptor opened on a link without O_SYMLINK is the target and the copy is a regular file. The
 * source's name does not matter: unlinking it first still clones what the descriptor holds. */
int test_main(void) {
    bsd2(SYS_mkdir, "tree", 0755);
    bsd2(SYS_mkdir, "tree/sub", 0750);
    mkfile("tree/a", "AAA");
    mkfile("tree/sub/b", "BBBBB");
    int t = bsd3(SYS_open, "tree", O_RDONLY, 0).ret;
    sysret r = bsd4(SYS_fclonefileat, t, AT_FDCWD, "copy", 0);
    check_ok("clone a directory tree", r);
    check("files and content", content_is("copy/a", "AAA") && content_is("copy/sub/b", "BBBBB"), 0);
    struct poly_stat64 st;
    bsd2(SYS_stat64, "copy/sub", &st);
    check("the subdirectory's mode 0750", (st.st_mode & 07777) == 0750, st.st_mode & 07777);
    mkfile("target", "T");
    bsd2(SYS_symlink, "target", "link");
    int l = bsd3(SYS_open, "link", O_SYMLINK | O_RDONLY, 0).ret;
    check_ok("clone an O_SYMLINK descriptor", bsd4(SYS_fclonefileat, l, AT_FDCWD, "linkcopy", 0));
    bsd2(SYS_lstat64, "linkcopy", &st);
    check("the copy is a symbolic link", (st.st_mode & S_IFMT) == S_IFLNK, st.st_mode);
    int f = bsd3(SYS_open, "link", O_RDONLY, 0).ret;
    check_ok("clone a descriptor opened through the link", bsd4(SYS_fclonefileat, f, AT_FDCWD, "filecopy", 0));
    bsd2(SYS_lstat64, "filecopy", &st);
    check("the copy is the target's file", (st.st_mode & S_IFMT) == S_IFREG && content_is("filecopy", "T"), st.st_mode);
    int s = bsd3(SYS_open, "target", O_RDONLY, 0).ret;
    bsd1(SYS_unlink, "target");
    check_ok("the source was unlinked: the descriptor still clones", bsd4(SYS_fclonefileat, s, AT_FDCWD, "ghost", 0));
    check("with the content", content_is("ghost", "T"), 0);
    return finish();
}
