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
/* rename, BSD 128, on the case-insensitive but case-preserving file system: renaming a file to the
 * same name in another case is allowed and changes the stored case (the directory listing shows the
 * new case, the inode stays the same); any spelling of the old name finds the file; renaming another
 * file onto a name that exists in a different case replaces that file's content and the destination entry
 * keeps its stored spelling (it is not duplicated). */
static int listed(const char *name) {
    int d = bsd3(SYS_open, ".", O_RDONLY, 0).ret;
    static char buf[8192];
    i64 base = 0;
    sysret r = bsd4(SYS_getdirentries64, d, buf, sizeof buf, &base);
    int found = 0;
    for (i64 off = 0; off < r.ret;) {
        struct poly_dirent64 *e = (void *)(buf + off);
        if (str_eq(e->d_name, name)) found = 1;
        off += e->d_reclen;
    }
    bsd1(SYS_close, d);
    return found;
}
int test_main(void) {
    mkfile("File.txt", "data");
    i64 ino = ino_of("File.txt");
    check("listed as created", listed("File.txt"), 0);
    sysret r = bsd2(SYS_rename, "File.txt", "file.txt");
    check("a case-only rename: 0", r.err == 0, r.err);
    check("the listing shows the new case", listed("file.txt") && !listed("File.txt"), 0);
    check("same inode, same content", ino_of("file.txt") == ino && content_is("file.txt", "data"), 0);
    check("any case finds it", content_is("FILE.TXT", "data") && content_is("File.TXT", "data"), 0);
    r = bsd2(SYS_rename, "FILE.TXT", "Final.txt");
    check("renaming through another spelling of the old name works", r.err == 0 && listed("Final.txt") && !listed("file.txt"), r.err);
    r = bsd2(SYS_rename, "final.TXT", "FINAL.TXT");
    check("a case-only rename given in another case of the old name", r.err == 0 && listed("FINAL.TXT") && !listed("Final.txt"), r.err);
    mkfile("other", "OTHER");
    i64 oino = ino_of("other");
    r = bsd2(SYS_rename, "other", "final.txt");
    check("a file renamed onto another case of an existing name: 0", r.err == 0, r.err);
    check("the content is the renamed file's", content_is("FINAL.TXT", "OTHER") && ino_of("FINAL.TXT") == oino, 0);
    check("the destination keeps its stored spelling FINAL.TXT", listed("FINAL.TXT") && !listed("final.txt"), 0);
    check("the source name is gone", !exists("other"), 0);
    bsd2(SYS_mkdir, "Dir", 0755);
    r = bsd2(SYS_rename, "Dir", "dir");
    check("a directory renamed to another case", r.err == 0 && listed("dir") && !listed("Dir"), r.err);
    return finish();
}
