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
/* clonefileat, BSD 462: modes and ownership. The copy of a file with mode 0600 has mode 0600; a source that
 * cannot be read (mode 0) is EACCES (13); a destination directory without write permission is
 * EACCES; a clone of an empty file is empty; of a file with a hole keeps the size; cloning does not
 * touch the source's times; the clone gets the caller as owner (uid and gid are the caller's);
 * with CLONE_NOOWNERCOPY (2) too. Cloning a file that is open for writing shows the bytes written so far. */
int test_main(void) {
    mkfile("empty", "");
    check_ok("clone an empty file", bsd6(SYS_clonefileat, AT_FDCWD, "empty", AT_FDCWD, "e2", 0, 0));
    check("it is empty", size_of("e2") == 0, size_of("e2"));
    mkfile("private", "secret");
    bsd2(SYS_chmod, "private", 0600);
    bsd6(SYS_clonefileat, AT_FDCWD, "private", AT_FDCWD, "p2", 0, 0);
    struct poly_stat64 st, a;
    bsd2(SYS_stat64, "p2", &st);
    check("mode 0600 is kept", (st.st_mode & 07777) == 0600, st.st_mode & 07777);
    check("owner is the caller", st.st_uid == (u32)bsd0(SYS_getuid).ret && st.st_gid == (u32)bsd0(SYS_getgid).ret, st.st_uid);
    check_ok("CLONE_NOOWNERCOPY", bsd6(SYS_clonefileat, AT_FDCWD, "private", AT_FDCWD, "p3", 2, 0));
    bsd2(SYS_stat64, "p3", &st);
    check("the owner is still the caller", st.st_uid == (u32)bsd0(SYS_getuid).ret, st.st_uid);
    mkfile("locked", "x");
    bsd2(SYS_chmod, "locked", 0);
    check_err("a source without read permission is EACCES", bsd6(SYS_clonefileat, AT_FDCWD, "locked", AT_FDCWD, "l2", 0, 0), EACCES);
    check("nothing was created", !exists("l2"), 0);
    bsd2(SYS_mkdir, "ro", 0555);
    check_err("a destination directory without write permission is EACCES", bsd6(SYS_clonefileat, AT_FDCWD, "private", AT_FDCWD, "ro/x", 0, 0), EACCES);
    mkfile("sparse", "");
    int fd = bsd3(SYS_open, "sparse", O_RDWR, 0).ret;
    bsd3(SYS_lseek, fd, 100000, SEEK_SET);
    bsd3(SYS_write, fd, "end", 3);
    bsd6(SYS_clonefileat, AT_FDCWD, "sparse", AT_FDCWD, "s2", 0, 0);
    check("a file with a hole keeps its size", size_of("s2") == 100003, size_of("s2"));
    bsd2(SYS_stat64, "sparse", &a);
    bsd6(SYS_clonefileat, AT_FDCWD, "sparse", AT_FDCWD, "s3", 0, 0);
    bsd2(SYS_stat64, "sparse", &st);
    check("cloning leaves the source's times alone", a.st_mtimespec.tv_nsec == st.st_mtimespec.tv_nsec && a.st_atimespec.tv_sec == st.st_atimespec.tv_sec, 0);
    int w = bsd3(SYS_open, "open_for_write", O_CREAT | O_RDWR, 0644).ret;
    bsd3(SYS_write, w, "so far", 6);
    bsd6(SYS_clonefileat, AT_FDCWD, "open_for_write", AT_FDCWD, "snap", 0, 0);
    bsd3(SYS_write, w, "+more", 5);
    check("a clone of a file open for writing has what was written before", content_is("snap", "so far") && content_is("open_for_write", "so far+more"), 0);
    return finish();
}
