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
/* renameatx_np, BSD 488 (fromfd, from, tofd, to, flags): rename with flags. RENAME_SWAP (2)
 * exchanges two existing names atomically: each name then has the other's content and inode; it
 * works for two files, two directories, and a file with a directory; a missing name on either side is
 * ENOENT (2); swapping a name with itself works (0). RENAME_EXCL (4) is EEXIST (17) when the destination
 * exists and a plain rename otherwise. Flags 2 and 4 together are EINVAL (22), and so is any bit other
 * than 1, 2, 4 (8 is EINVAL; 1 is accepted and acts as a plain rename). */
int test_main(void) {
    mkfile("a", "AAA");
    mkfile("b", "BB");
    i64 ia = ino_of("a"), ib = ino_of("b");
    sysret r = bsd6(SYS_renameatx_np, AT_FDCWD, "a", AT_FDCWD, "b", RENAME_SWAP, 0);
    check("swap two files: 0", r.err == 0 && r.ret == 0, r.err);
    check("a has b's content and inode", content_is("a", "BB") && ino_of("a") == ib, 0);
    check("b has a's content and inode", content_is("b", "AAA") && ino_of("b") == ia, 0);
    check_err("swap with a missing destination is ENOENT", bsd6(SYS_renameatx_np, AT_FDCWD, "a", AT_FDCWD, "nosuch", RENAME_SWAP, 0), ENOENT);
    check_err("swap with a missing source is ENOENT", bsd6(SYS_renameatx_np, AT_FDCWD, "nosuch", AT_FDCWD, "a", RENAME_SWAP, 0), ENOENT);
    check("nothing changed", content_is("a", "BB") && content_is("b", "AAA") && !exists("nosuch"), 0);
    r = bsd6(SYS_renameatx_np, AT_FDCWD, "a", AT_FDCWD, "a", RENAME_SWAP, 0);
    value("swapping a name with itself: errno", r.err);
    bsd2(SYS_mkdir, "d1", 0755);
    bsd2(SYS_mkdir, "d2", 0755);
    mkfile("d1/in1", "1");
    mkfile("d2/in2", "2");
    check_ok("swap two directories", bsd6(SYS_renameatx_np, AT_FDCWD, "d1", AT_FDCWD, "d2", RENAME_SWAP, 0));
    check("their contents changed places", exists("d1/in2") && exists("d2/in1") && !exists("d1/in1"), 0);
    r = bsd6(SYS_renameatx_np, AT_FDCWD, "d1", AT_FDCWD, "a", RENAME_SWAP, 0);
    check_ok("swap a directory with a file", r);
    struct poly_stat64 st;
    bsd2(SYS_stat64, "a", &st);
    check("now a is the directory", (st.st_mode & S_IFMT) == S_IFDIR && exists("a/in2"), st.st_mode);
    bsd2(SYS_stat64, "d1", &st);
    check("and d1 the file", (st.st_mode & S_IFMT) == S_IFREG && content_is("d1", "BB"), st.st_mode);
    int fd = bsd3(SYS_open, "b", O_RDWR, 0).ret;
    bsd2(SYS_mkdir, "dd", 0755);
    mkfile("c", "C");
    bsd6(SYS_renameatx_np, AT_FDCWD, "b", AT_FDCWD, "c", RENAME_SWAP, 0);
    bsd3(SYS_write, fd, "!", 1);
    check("a descriptor follows its file through a swap", content_is("c", "!AA"), 0);
    check_err("SWAP and EXCL together are EINVAL", bsd6(SYS_renameatx_np, AT_FDCWD, "a", AT_FDCWD, "c", RENAME_SWAP | RENAME_EXCL, 0), EINVAL);
    check_err("flag 8 is EINVAL", bsd6(SYS_renameatx_np, AT_FDCWD, "a", AT_FDCWD, "c", 8, 0), EINVAL);
    check_err("flag 0x100 is EINVAL", bsd6(SYS_renameatx_np, AT_FDCWD, "a", AT_FDCWD, "c", 0x100, 0), EINVAL);
    return finish();
}
