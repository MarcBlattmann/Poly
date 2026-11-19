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
/* setattrlist, BSD 221 (path, attrlist, buffer, size, options): sets attributes from a packed
 * buffer in bitmap order. ATTR_CMN_MODTIME (0x400) takes a struct timespec (16 bytes), ATTR_CMN_ACCTIME
 * (0x1000) too, both together in that order (modification time first); nanoseconds are kept
 * (atime 2000000000.000000005); ATTR_CMN_ACCESSMASK (0x20000) takes a u32 mode, ATTR_CMN_FLAGS
 * (0x40000) a u32 of file flags (UF_HIDDEN 0x8000). Results are seen by stat. The call returns 0. */
#define ATTR_CMN_MODTIME 0x400u
#define ATTR_CMN_ACCTIME 0x1000u
#define ATTR_CMN_ACCESSMASK 0x20000u
#define ATTR_CMN_FLAGS 0x40000u
int test_main(void) {
    mkfile("file", "x");
    struct poly_stat64 st;
    struct poly_timespec ts = {1000000000, 0};
    struct attrlist_t m = {ATTR_BIT_MAP_COUNT, 0, ATTR_CMN_MODTIME, 0, 0, 0, 0};
    sysret r = bsd6(SYS_setattrlist, "file", &m, &ts, sizeof ts, 0, 0);
    check("set the modification time: 0", r.err == 0 && r.ret == 0, r.err);
    bsd2(SYS_stat64, "file", &st);
    check("stat shows it", st.st_mtimespec.tv_sec == 1000000000 && st.st_mtimespec.tv_nsec == 0, st.st_mtimespec.tv_sec);
    struct attrlist_t ac = {ATTR_BIT_MAP_COUNT, 0, ATTR_CMN_ACCTIME, 0, 0, 0, 0};
    struct poly_timespec at = {2000000000, 5};
    check_ok("set the access time", bsd6(SYS_setattrlist, "file", &ac, &at, sizeof at, 0, 0));
    bsd2(SYS_stat64, "file", &st);
    check("with its nanoseconds", st.st_atimespec.tv_sec == 2000000000 && st.st_atimespec.tv_nsec == 5, st.st_atimespec.tv_nsec);
    check("the modification time is unchanged", st.st_mtimespec.tv_sec == 1000000000, st.st_mtimespec.tv_sec);
    struct attrlist_t both = {ATTR_BIT_MAP_COUNT, 0, ATTR_CMN_MODTIME | ATTR_CMN_ACCTIME, 0, 0, 0, 0};
    struct poly_timespec two[2] = {{1100000000, 0}, {1200000000, 0}};
    check_ok("both at once", bsd6(SYS_setattrlist, "file", &both, two, sizeof two, 0, 0));
    bsd2(SYS_stat64, "file", &st);
    check("the modification time comes first in the buffer", st.st_mtimespec.tv_sec == 1100000000 && st.st_atimespec.tv_sec == 1200000000, st.st_mtimespec.tv_sec);
    struct attrlist_t am = {ATTR_BIT_MAP_COUNT, 0, ATTR_CMN_ACCESSMASK, 0, 0, 0, 0};
    u32 mode = 0640;
    check_ok("set the mode", bsd6(SYS_setattrlist, "file", &am, &mode, 4, 0, 0));
    bsd2(SYS_stat64, "file", &st);
    check("stat shows 0640", (st.st_mode & 07777) == 0640, st.st_mode & 07777);
    check("and it is still a regular file", (st.st_mode & S_IFMT) == S_IFREG, st.st_mode);
    struct attrlist_t fl = {ATTR_BIT_MAP_COUNT, 0, ATTR_CMN_FLAGS, 0, 0, 0, 0};
    u32 flg = 0x8000;
    check_ok("set the file flags to UF_HIDDEN", bsd6(SYS_setattrlist, "file", &fl, &flg, 4, 0, 0));
    bsd2(SYS_stat64, "file", &st);
    check("st_flags is 0x8000", st.st_flags == 0x8000, st.st_flags);
    flg = 0;
    bsd6(SYS_setattrlist, "file", &fl, &flg, 4, 0, 0);
    bsd2(SYS_stat64, "file", &st);
    check("and can be cleared", st.st_flags == 0, st.st_flags);
    return finish();
}
