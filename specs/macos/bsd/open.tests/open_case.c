#include "poly_mtest.h"
/* open, BSD 5, on the case-insensitive but case-preserving file system: a name is found whatever
 * its case ("file.TXT" finds "File.txt"); creating a name that differs only in case from an existing one
 * opens the existing file (O_CREAT) or is EEXIST (O_CREAT|O_EXCL); the directory listing keeps
 * the case the file was created with; F_GETPATH of a descriptor shows the stored case, not the case that was
 * asked for; stat through any spelling gives the same inode; unlink and rmdir work through any
 * spelling; case differences inside path components do not matter; two links whose names differ only in
 * case are the same name (EEXIST). */
#define SYS_link 9
#ifndef F_GETPATH
#define F_GETPATH 50
#endif
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
    int fd = bsd3(SYS_open, "File.txt", O_CREAT | O_RDWR, 0644).ret;
    bsd3(SYS_write, fd, "abc", 3);
    sysret r = bsd3(SYS_open, "file.TXT", O_RDONLY, 0);
    check("another case finds the file", r.err == 0, r.err);
    char buf[8];
    memset(buf, 0, sizeof buf);
    bsd3(SYS_read, r.ret, buf, 7);
    check("and it is the same file", str_eq(buf, "abc"), 0);
    int other = (int)r.ret;
    r = bsd3(SYS_open, "FILE.TXT", O_CREAT | O_EXCL | O_RDWR, 0644);
    check_err("O_CREAT|O_EXCL on another case is EEXIST", r, EEXIST);
    r = bsd3(SYS_open, "FILE.TXT", O_CREAT | O_RDWR, 0644);
    check("O_CREAT on another case opens the existing file", r.err == 0, r.err);
    check("no second file was created: one entry, with the first spelling", listed("File.txt") && !listed("FILE.TXT"), 0);
    char path[1100];
    memset(path, 0, sizeof path);
    bsd3(SYS_fcntl, other, F_GETPATH, path);
    size_t n = str_len(path);
    check("F_GETPATH shows the stored case, not the asked one", n >= 9 && str_eq(path + n - 9, "/File.txt"), n);
    struct poly_stat64 a, b;
    bsd2(SYS_stat64, "File.txt", &a);
    check("stat through another spelling: same inode", bsd2(SYS_stat64, "fILE.tXt", &b).err == 0 && a.st_ino == b.st_ino, 0);
    check_err("link to another case of the same name is EEXIST", bsd2(SYS_link, "File.txt", "FILE.TXT"), EEXIST);
    check_ok("link under a new name", bsd2(SYS_link, "FILE.TXT", "Other"));
    struct poly_stat64 c;
    bsd2(SYS_stat64, "Other", &c);
    check("a link is a second name for the same inode", c.st_ino == a.st_ino && c.st_nlink == 2, c.st_nlink);
    check_ok("unlink through another spelling", bsd1(SYS_unlink, "OTHER"));
    check("the link is gone, the file stays", !listed("Other") && listed("File.txt"), 0);
    bsd2(SYS_mkdir, "Dir", 0755);
    bsd1(SYS_close, bsd3(SYS_open, "dir/Inner", O_CREAT | O_RDWR, 0644).ret);
    check("case differences inside a path do not matter", bsd2(SYS_stat64, "DIR/inner", &b).err == 0, 0);
    check_err("mkdir of another case of a directory is EEXIST", bsd2(SYS_mkdir, "DIR", 0755), EEXIST);
    check_err("mkdir of another case of a file's name is EEXIST", bsd2(SYS_mkdir, "FILE.txt", 0755), EEXIST);
    r = bsd3(SYS_open, "DIR/INNER/x", O_RDONLY, 0);
    check_err("a file used as a directory is ENOTDIR whatever the case", r, ENOTDIR);
    check_err("rmdir through another spelling of a non-empty directory is ENOTEMPTY", bsd1(SYS_rmdir, "dIR"), ENOTEMPTY);
    bsd1(SYS_unlink, "DIR/inner");
    check_ok("and of an empty one works", bsd1(SYS_rmdir, "dIR"));
    check("it is gone", !listed("Dir"), 0);
    return finish();
}
