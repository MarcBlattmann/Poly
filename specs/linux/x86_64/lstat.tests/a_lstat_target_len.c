/* lstat: st_size of a symbolic link is the byte length of its target text
 * (no NUL), from 1 up to the 4095-byte maximum, whatever the bytes are. */
#include "a_sys.h"

#define D "/tmp/a_lstat_target_len"

static i64 lstat(const char *p, struct a_stat *st) { return syscall2(SYS_lstat, p, st); }
static i64 symlink(const char *t, const char *l) { return syscall3(SYS_symlinkat, t, AT_FDCWD, l); }

static char target[4200];

static i64 size_for(i64 n) {
    struct a_stat st;
    for (i64 i = 0; i < n; i++) target[i] = (i % 7 == 6) ? '/' : 'x';
    target[n] = 0;
    i64 r = symlink(target, D "/l");
    if (r < 0) return r;
    r = lstat(D "/l", &st);
    a_unlink(D "/l");
    return r < 0 ? r : st.st_size;
}

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("len_1", size_for(1), 1);
    expect("len_59", size_for(59), 59);
    expect("len_60", size_for(60), 60);
    expect("len_61", size_for(61), 61);
    expect("len_255", size_for(255), 255);
    expect("len_256", size_for(256), 256);
    expect("len_1000", size_for(1000), 1000);
    expect("len_4095", size_for(4095), 4095);
    expect("len_4096_refused", size_for(4096), -ENAMETOOLONG);
    expect("utf8_link", symlink("\xc3\xa9t\xc3\xa9", D "/u"), 0);
    expect("utf8_size_bytes", (lstat(D "/u", &st), st.st_size), 5);
    expect("abs_link", symlink("/", D "/r"), 0);
    expect("abs_size", (lstat(D "/r", &st), st.st_size), 1);
    expect("abs_slash_follows", lstat(D "/r/", &st), 0);
    expect("abs_slash_is_dir", st.st_mode & S_IFMT, S_IFDIR);
    expect("cleanup_u", a_unlink(D "/u"), 0);
    expect("cleanup_r", a_unlink(D "/r"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
