#include "poly_mtest.h"
/* getattrlist, BSD 220: attribute values agree with stat. Asked one at a time (buffer: u32 length, then the
 * value): ATTR_CMN_FILEID (0x02000000) is the inode; ATTR_CMN_OBJTYPE VREG 1, VDIR 2, VLNK 5, with FSOPT_NOFOLLOW
 * (1) a link is VLNK, without it the target's type; ATTR_CMN_MODTIME (0x400), ATTR_CMN_CRTIME (0x200) and
 * ATTR_CMN_ACCTIME (0x1000) are timespecs equal to stat's; ATTR_CMN_ACCESSMASK (0x20000) is the mode including
 * the file type bits; ATTR_CMN_FLAGS (0x40000) is st_flags; ATTR_CMN_OWNERID (0x8000) and GRPID (0x10000)
 * the owner; ATTR_FILE_LINKCOUNT (1) the link count; ATTR_FILE_DATALENGTH (0x200) the size; ATTR_FILE_ALLOCSIZE
 * (0x4) a multiple of the block size at least the size; ATTR_DIR_LINKCOUNT (1) is 1 for a directory with one subdirectory, ATTR_DIR_ENTRYCOUNT (2) the number of entries
 * (not counting . and ..); a missing file is ENOENT; the name attribute keeps the stored case. */
#define SYS_link 9
struct attrlist_t { u16 bitmapcount; u16 reserved; u32 commonattr, volattr, dirattr, fileattr, forkattr; };
static sysret ga(const char *p, u32 cmn, u32 dir, u32 file, void *buf, int size, int opt) {
    struct attrlist_t al = {5, 0, cmn, 0, dir, file, 0};
    return bsd6(SYS_getattrlist, p, &al, buf, size, opt, 0);
}
int test_main(void) {
    int fd = bsd3(SYS_open, "Data.bin", O_CREAT | O_RDWR, 0640).ret;
    bsd3(SYS_write, fd, "0123456789", 10);
    bsd2(SYS_mkdir, "dir", 0750);
    bsd1(SYS_close, bsd3(SYS_open, "dir/a", O_CREAT | O_RDWR, 0644).ret);
    bsd1(SYS_close, bsd3(SYS_open, "dir/b", O_CREAT | O_RDWR, 0644).ret);
    bsd2(SYS_mkdir, "dir/c", 0755);
    bsd2(SYS_symlink, "Data.bin", "link");
    struct poly_stat64 st;
    bsd2(SYS_stat64, "Data.bin", &st);
    char buf[64];
    u32 *u = (u32 *)buf;
    sysret r = ga("Data.bin", 0x02000000u, 0, 0, buf, sizeof buf, 0);
    check("FILEID: length 12 and the inode", r.err == 0 && u[0] == 12 && *(u64 *)(buf + 4) == st.st_ino, r.err);
    r = ga("Data.bin", 0x400, 0, 0, buf, sizeof buf, 0);
    check("MODTIME equals stat's", r.err == 0 && u[0] == 20 && *(i64 *)(buf + 4) == st.st_mtimespec.tv_sec && *(i64 *)(buf + 12) == st.st_mtimespec.tv_nsec, r.err);
    r = ga("Data.bin", 0x200, 0, 0, buf, sizeof buf, 0);
    check("CRTIME equals stat's birth time", r.err == 0 && *(i64 *)(buf + 4) == st.st_birthtimespec.tv_sec && *(i64 *)(buf + 12) == st.st_birthtimespec.tv_nsec, r.err);
    r = ga("Data.bin", 0x20000, 0, 0, buf, sizeof buf, 0);
    check("ACCESSMASK is the mode with the type bits (0100640)", r.err == 0 && u[0] == 8 && u[1] == st.st_mode && st.st_mode == 0100640, u[1]);
    r = ga("Data.bin", 0x40000, 0, 0, buf, sizeof buf, 0);
    check("FLAGS equals st_flags", r.err == 0 && u[1] == st.st_flags, u[1]);
    r = ga("Data.bin", 0x8000, 0, 0, buf, sizeof buf, 0);
    check("OWNERID is the owner", r.err == 0 && u[1] == st.st_uid, u[1]);
    r = ga("Data.bin", 0x10000, 0, 0, buf, sizeof buf, 0);
    check("GRPID is the group", r.err == 0 && u[1] == st.st_gid, u[1]);
    r = ga("Data.bin", 0, 0, 1, buf, sizeof buf, 0);
    check("FILE LINKCOUNT is 1", r.err == 0 && u[0] == 8 && u[1] == 1, u[1]);
    bsd2(SYS_link, "Data.bin", "hard");
    r = ga("Data.bin", 0, 0, 1, buf, sizeof buf, 0);
    check("and 2 after a hard link", r.err == 0 && u[1] == 2, u[1]);
    r = ga("Data.bin", 0, 0, 0x200, buf, sizeof buf, 0);
    check("DATALENGTH is the size 10", r.err == 0 && u[0] == 12 && *(i64 *)(buf + 4) == 10, r.err);
    r = ga("Data.bin", 0, 0, 0x4, buf, sizeof buf, 0);
    check("ALLOCSIZE is at least the size and a multiple of 4096", r.err == 0 && *(i64 *)(buf + 4) >= 10 && *(i64 *)(buf + 4) % 4096 == 0, *(i64 *)(buf + 4));
    r = ga("dir", 0, 2, 0, buf, sizeof buf, 0);
    check("DIR ENTRYCOUNT (0x2) is 3 (a, b, c)", r.err == 0 && u[0] == 8 && u[1] == 3, u[1]);
    r = ga("dir", 0, 1, 0, buf, sizeof buf, 0);
    check("DIR LINKCOUNT (0x1) is 1 here (one subdirectory)", r.err == 0 && u[0] == 8 && u[1] == 1, u[1]);
    r = ga("link", 8, 0, 0, buf, sizeof buf, 1);
    check("OBJTYPE with FSOPT_NOFOLLOW: VLNK", r.err == 0 && u[1] == 5, u[1]);
    r = ga("link", 8, 0, 0, buf, sizeof buf, 0);
    check("OBJTYPE following the link: VREG", r.err == 0 && u[1] == 1, u[1]);
    r = ga("dir", 8, 0, 0, buf, sizeof buf, 0);
    check("OBJTYPE of a directory: VDIR", r.err == 0 && u[1] == 2, u[1]);
    r = ga("DATA.BIN", 1, 0, 0, buf, sizeof buf, 0);
    check("NAME through another case: the stored \"Data.bin\"", r.err == 0 && u[2] == 9 && str_eq(buf + 4 + (i32)u[1], "Data.bin"), r.err);
    check_err("a missing file is ENOENT", ga("nosuch", 8, 0, 0, buf, sizeof buf, 0), ENOENT);
    return finish();
}
