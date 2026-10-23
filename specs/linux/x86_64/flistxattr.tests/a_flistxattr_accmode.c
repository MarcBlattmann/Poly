/* flistxattr works through any descriptor of a file or directory,
 * whatever its access mode: O_RDONLY, O_WRONLY, O_RDWR, O_WRONLY|O_APPEND,
 * a directory opened O_DIRECTORY or plain O_RDONLY, a dup'd descriptor
 * after the original is closed, a descriptor opened through a symbolic link
 * (lists the target) and a reopen through /proc/self/fd/N. */
#include "a_sys.h"

#define SYS_setxattr 188
#define SYS_flistxattr 196
#define D "/tmp/a_flistxattr_accmode"

static char buf[256];

static i64 flist(i64 fd) { return syscall3(SYS_flistxattr, fd, buf, sizeof buf); }

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_set", syscall5(SYS_setxattr, D "/f", "user.a", "1", 1, 0), 0);
    expect("setup_set_dir", syscall5(SYS_setxattr, D, "user.dir", "", 0, 0), 0);
    expect("setup_link", syscall2(SYS_symlink, "f", D "/l"), 0);
    i64 rd = a_open(D "/f", O_RDONLY, 0);
    i64 wr = a_open(D "/f", O_WRONLY, 0);
    i64 rw = a_open(D "/f", O_RDWR, 0);
    i64 ap = a_open(D "/f", O_WRONLY | O_APPEND, 0);
    check("setup_open", rd >= 0 && wr >= 0 && rw >= 0 && ap >= 0, rd);
    expect("rdonly", flist(rd), 7);
    check("rdonly_bytes", mem_eq(buf, "user.a\0", 7), 0);
    expect("wronly", flist(wr), 7);
    expect("rdwr", flist(rw), 7);
    expect("append", flist(ap), 7);
    i64 dd = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    expect("o_directory", flist(dd), 9);
    check("o_directory_bytes", mem_eq(buf, "user.dir\0", 9), 0);
    i64 dr = a_open(D, O_RDONLY, 0);
    expect("dir_rdonly", flist(dr), 9);
    i64 du = syscall1(SYS_dup, rd);
    check("setup_dup", du >= 0, du);
    expect("setup_close_original", a_close(rd), 0);
    expect("dup_after_close", flist(du), 7);
    i64 lf = a_open(D "/l", O_RDONLY, 0);
    expect("through_symlink", flist(lf), 7);
    check("through_symlink_bytes", mem_eq(buf, "user.a\0", 7), 0);
    char path[64], num[24];
    str_cpy(path, "/proc/self/fd/");
    a_utoa(num, (u64)wr);
    str_cpy(path + str_len(path), num);
    i64 re = a_open(path, O_RDONLY, 0);
    check("setup_reopen", re >= 0, re);
    expect("proc_reopen", flist(re), 7);
    a_close(re);
    a_close(lf);
    a_close(du);
    a_close(dr);
    a_close(dd);
    a_close(ap);
    a_close(rw);
    a_close(wr);
    expect("cleanup_l", a_unlink(D "/l"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
