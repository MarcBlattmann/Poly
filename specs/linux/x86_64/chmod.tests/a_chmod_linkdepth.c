/* chmod: a chain of 40 symbolic links is followed; 41 is ELOOP. Links in
 * directory components count toward the same limit. */
#include "a_sys.h"

#define D "/tmp/a_chmod_linkdepth"

static char name[64], target[64];

static char *nm(char *buf, int i) {
    char num[24];
    a_utoa(num, (u64)i);
    return str_cat2(buf, D "/l", num);
}

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/l0", "", 0644), 0);
    int ok = 1;
    for (int i = 1; i <= 41; i++) ok &= syscall3(SYS_symlinkat, nm(target, i - 1), AT_FDCWD, nm(name, i)) == 0;
    check("setup_chain", ok, 0);
    expect("depth_1", syscall2(SYS_chmod, nm(name, 1), 0600), 0);
    expect("depth_39", syscall2(SYS_chmod, nm(name, 39), 0640), 0);
    expect("depth_40", syscall2(SYS_chmod, nm(name, 40), 0604), 0);
    struct a_stat st;
    expect("depth_40_applied", a_stat_path(D "/l0", &st) == 0 && st.st_mode == (S_IFREG | 0604), 1);
    expect("depth_41_eloop", syscall2(SYS_chmod, nm(name, 41), 0600), -ELOOP);
    expect("depth_41_unchanged", a_stat_path(D "/l0", &st) == 0 && st.st_mode == (S_IFREG | 0604), 1);

    /* A directory reached through a link, then a chain to the file. */
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_sublink", syscall3(SYS_symlinkat, "sub", AT_FDCWD, D "/sl"), 0);
    expect("setup_subfile", syscall3(SYS_symlinkat, nm(target, 39), AT_FDCWD, D "/sub/c"), 0);
    expect("dir_link_plus_40", syscall2(SYS_chmod, D "/sl/c", 0600), -ELOOP);
    expect("no_dir_link_40", syscall2(SYS_chmod, D "/sub/c", 0600), 0);

    ok = a_unlink(D "/sub/c") == 0 && a_rmdir(D "/sub") == 0 && a_unlink(D "/sl") == 0;
    for (int i = 0; i <= 41; i++) ok &= a_unlink(nm(name, i)) == 0;
    check("cleanup_names", ok, 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
