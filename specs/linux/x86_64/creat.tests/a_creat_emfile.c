/* creat(2) with the descriptor table full (RLIMIT_NOFILE lowered): EMFILE,
 * reported before the path is looked at, and nothing is created. */
#include "a_sys.h"

#define SYS_creat 85
#define D "/tmp/a_creat_emfile"

static i64 cr(const char *p, i64 mode) { return syscall2(SYS_creat, p, mode); }

int test_main(void) {
    struct a_rlimit old, low;
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    expect("setup_file", a_make_file(D "/keep", "data", 0644), 0);
    expect("getrlimit", syscall2(SYS_getrlimit, RLIMIT_NOFILE, &old), 0);
    low.rlim_cur = 6;
    low.rlim_max = old.rlim_max;
    expect("lower_limit", syscall2(SYS_setrlimit, RLIMIT_NOFILE, &low), 0);

    expect("fill_3", cr(D "/a3", 0644), 3);
    expect("fill_4", cr(D "/a4", 0644), 4);
    expect("fill_5", cr(D "/a5", 0644), 5);
    expect("full_new_emfile", cr(D "/new", 0644), -EMFILE);
    expect("full_new_not_created", a_stat_path(D "/new", &st), -ENOENT);
    expect("full_existing_emfile", cr(D "/keep", 0644), -EMFILE);
    expect("full_existing_not_truncated", a_stat_path(D "/keep", &st) == 0 ? (i64)st.st_size : -1, 4);
    expect("full_missing_dir_emfile", cr(D "/no/x", 0644), -EMFILE);
    expect("full_bad_pointer_emfile", cr((const char *)0, 0644), -EMFILE);

    a_close(4);
    expect("after_close_gets_4", cr(D "/new", 0644), 4);
    expect("full_again_emfile", cr(D "/new2", 0644), -EMFILE);

    for (int i = 3; i < 6; i++) a_close(i);
    expect("restore_limit", syscall2(SYS_setrlimit, RLIMIT_NOFILE, &old), 0);

    const char *names[] = {D "/keep", D "/a3", D "/a4", D "/a5", D "/new"};
    for (int i = 0; i < 5; i++) expect("cleanup_unlink", a_unlink(names[i]), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
