/* symlinkat: the target text is stored as given (relative, absolute,
 * dangling); the name must not exist; an empty target is ENOENT. */
#include "a_sys.h"

#define D "/tmp/a_symlinkat_basic"

static char buf[256];

static i64 rl(const char *p) { return syscall4(SYS_readlinkat, AT_FDCWD, p, buf, sizeof buf); }

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_file", a_make_file(D "/f", "data", 0644), 0);
    expect("relative", syscall3(SYS_symlinkat, "f", AT_FDCWD, D "/rel"), 0);
    expect("relative_text", rl(D "/rel"), 1);
    expect("absolute", syscall3(SYS_symlinkat, D "/f", AT_FDCWD, D "/abs"), 0);
    expect("absolute_text_length", rl(D "/abs"), (i64)str_len(D "/f"));
    check("absolute_text", mem_eq(buf, D "/f", str_len(D "/f")), 0);
    expect("dangling_allowed", syscall3(SYS_symlinkat, "../nowhere/x", AT_FDCWD, D "/dang"), 0);
    expect("dangling_text", rl(D "/dang"), 12);
    check("dangling_text_kept", mem_eq(buf, "../nowhere/x", 12), 0);
    expect("weird_text_kept", syscall3(SYS_symlinkat, "a//b/./c/", AT_FDCWD, D "/weird"), 0);
    expect("weird_text_length", rl(D "/weird"), 9);
    check("weird_text", mem_eq(buf, "a//b/./c/", 9), 0);
    expect("existing_name_eexist", syscall3(SYS_symlinkat, "x", AT_FDCWD, D "/f"), -EEXIST);
    expect("existing_symlink_eexist", syscall3(SYS_symlinkat, "x", AT_FDCWD, D "/rel"), -EEXIST);
    expect("empty_target_enoent", syscall3(SYS_symlinkat, "", AT_FDCWD, D "/empty"), -ENOENT);
    expect("missing_parent_enoent", syscall3(SYS_symlinkat, "x", AT_FDCWD, D "/no/l"), -ENOENT);
    expect("parent_is_file_enotdir", syscall3(SYS_symlinkat, "x", AT_FDCWD, D "/f/l"), -ENOTDIR);
    expect("bad_target_efault", syscall3(SYS_symlinkat, (const char *)8, AT_FDCWD, D "/bad"), -EFAULT);
    expect("bad_name_efault", syscall3(SYS_symlinkat, "x", AT_FDCWD, (const char *)8), -EFAULT);
    expect("lstat_symlink", a_lstat_path(D "/abs", &st), 0);
    expect("symlink_mode", st.st_mode, S_IFLNK | 0777);
    expect("symlink_nlink", st.st_nlink, 1);
    i64 fd = a_open(D "/abs", O_RDONLY, 0);
    expect("open_through_absolute", a_read(fd, buf, sizeof buf), 4);
    a_close(fd);
    expect("old_symlink_syscall", syscall2(SYS_symlink, "f", D "/old"), 0);
    expect("old_symlink_text", rl(D "/old"), 1);
    const char *names[] = {D "/rel", D "/abs", D "/dang", D "/weird", D "/old", D "/f"};
    int ok = 1;
    for (int i = 0; i < 6; i++) ok &= a_unlink(names[i]) == 0;
    check("cleanup_names", ok, 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
