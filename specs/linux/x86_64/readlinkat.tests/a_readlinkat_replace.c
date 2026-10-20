/* readlinkat: links rewritten in place keep only the new text; a link
 * with a long relative text resolves; st_size of a link is its text
 * length. */
#include "a_sys.h"

#define D "/tmp/a_readlinkat_proc"

static i64 rl(const char *p, char *buf, i64 n) { return syscall4(SYS_readlinkat, AT_FDCWD, p, buf, n); }

int test_main(void) {
    char buf[128];
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("make_link", syscall3(SYS_symlinkat, "first-target-text", AT_FDCWD, D "/l"), 0);
    expect("first_text", rl(D "/l", buf, sizeof buf), 17);
    a_lstat_path(D "/l", &st);
    expect("st_size_is_text_length", st.st_size, 17);
    /* Replace it with a shorter one through a rename. */
    expect("make_tmp", syscall3(SYS_symlinkat, "short", AT_FDCWD, D "/tmp"), 0);
    expect("rename_over", syscall5(SYS_renameat2, AT_FDCWD, D "/tmp", AT_FDCWD, D "/l", 0), 0);
    memset(buf, 'z', sizeof buf);
    expect("new_text_length", rl(D "/l", buf, sizeof buf), 5);
    check("new_text_only", mem_eq(buf, "short", 5) && buf[5] == 'z', buf[5]);
    /* Relative text with many components. */
    expect("setup_deep", a_mkdir(D "/a", 0700), 0);
    expect("setup_deep2", a_mkdir(D "/a/b", 0700), 0);
    expect("setup_target", a_make_file(D "/a/b/t", "ok", 0644), 0);
    expect("deep_link", syscall3(SYS_symlinkat, "./a/../a/b/./t", AT_FDCWD, D "/deep"), 0);
    expect("deep_text", rl(D "/deep", buf, sizeof buf), 14);
    expect("deep_resolves", a_stat_path(D "/deep", &st) == 0 ? st.st_size : -1, 2);
    expect("cleanup_t", a_unlink(D "/a/b/t"), 0);
    expect("cleanup_b", a_rmdir(D "/a/b"), 0);
    expect("cleanup_a", a_rmdir(D "/a"), 0);
    expect("cleanup_deep", a_unlink(D "/deep"), 0);
    expect("cleanup_l", a_unlink(D "/l"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
