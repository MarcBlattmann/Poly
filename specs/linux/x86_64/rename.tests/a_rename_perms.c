/* rename and permissions: the caller needs write and search permission
 * on the directory it removes the old name from and on the one it adds
 * the new name to, and search permission on every directory on the
 * way; moving a directory to another parent also needs write permission
 * on the directory itself. Without them: EACCES, unless the caller is
 * root (who passes these checks). A missing old name in a writable-less
 * directory is still ENOENT. The file's own mode does not matter. */
#include "a_sys.h"

#define D "/tmp/a_rename_perms"

static i64 rn(const char *a, const char *b) { return syscall2(SYS_rename, a, b); }
static i64 chmod(const char *p, i64 m) { return syscall3(SYS_fchmodat, AT_FDCWD, p, m); }
static int exists(const char *p) {
    struct a_stat st;
    return a_lstat_path(p, &st) == 0;
}

int test_main(void) {
    int root = syscall0(SYS_geteuid) == 0;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_ro", a_mkdir(D "/ro", 0755), 0);
    expect("setup_ro_f", a_make_file(D "/ro/f", "", 0644), 0);
    expect("setup_f", a_make_file(D "/f", "", 0644), 0);
    expect("chmod_ro_555", chmod(D "/ro", 0555), 0);

    expect("out_of_read_only_dir", rn(D "/ro/f", D "/g"), root ? 0 : -EACCES);
    if (root) expect("root_put_back", rn(D "/g", D "/ro/f"), 0);
    expect("into_read_only_dir", rn(D "/f", D "/ro/f2"), root ? 0 : -EACCES);
    if (root) expect("root_take_out", rn(D "/ro/f2", D "/f"), 0);
    expect("within_read_only_dir", rn(D "/ro/f", D "/ro/g"), root ? 0 : -EACCES);
    if (root) expect("root_undo", rn(D "/ro/g", D "/ro/f"), 0);
    expect("missing_in_read_only_dir", rn(D "/ro/none", D "/z"), -ENOENT);
    expect("over_file_in_read_only_dir", rn(D "/f", D "/ro/f"), root ? 0 : -EACCES);
    if (root) expect("root_recreate_f", a_make_file(D "/f", "", 0644), 0);
    expect("ro_f_still_there", exists(D "/ro/f"), 1);
    expect("f_still_there", exists(D "/f"), 1);

    /* Write and search without read is enough. */
    expect("chmod_ro_300", chmod(D "/ro", 0300), 0);
    expect("write_search_only_ok", rn(D "/ro/f", D "/ro/g"), 0);
    /* Read and write without search is not. */
    expect("chmod_ro_600", chmod(D "/ro", 0600), 0);
    expect("no_search", rn(D "/ro/g", D "/ro/f"), root ? 0 : -EACCES);
    expect("no_search_missing", rn(D "/ro/none", D "/z"), root ? -ENOENT : -EACCES);
    expect("no_search_target", rn(D "/f", D "/ro/h"), root ? 0 : -EACCES);
    expect("chmod_ro_755", chmod(D "/ro", 0755), 0);
    if (root) {
        expect("root_undo_f", rn(D "/ro/f", D "/ro/g"), 0);
        expect("root_undo_h", rn(D "/ro/h", D "/f"), 0);
    }

    /* A mode-0 file can be renamed: only the directories count. */
    expect("chmod_f_000", chmod(D "/f", 0), 0);
    expect("mode_0_file", rn(D "/f", D "/f0"), 0);

    /* A read-only directory: renamed in place fine, moved elsewhere not. */
    expect("setup_p", a_mkdir(D "/p", 0755), 0);
    expect("setup_k", a_mkdir(D "/p/k", 0555), 0);
    expect("ro_dir_same_parent", rn(D "/p/k", D "/p/k2"), 0);
    expect("ro_dir_other_parent", rn(D "/p/k2", D "/k"), root ? 0 : -EACCES);
    if (root) expect("root_move_back", rn(D "/k", D "/p/k2"), 0);
    expect("setup_rf", a_make_file(D "/p/rf", "", 0444), 0);
    expect("ro_file_other_parent", rn(D "/p/rf", D "/rf"), 0);
    expect("chmod_k_755", chmod(D "/p/k2", 0755), 0);
    expect("rw_dir_other_parent", rn(D "/p/k2", D "/k"), 0);

    /* A sticky directory: the caller's own files move freely. */
    expect("setup_sticky", a_mkdir(D "/st", 0755), 0);
    expect("chmod_sticky", chmod(D "/st", 01777), 0);
    expect("setup_sticky_f", a_make_file(D "/st/f", "", 0644), 0);
    expect("own_file_in_sticky", rn(D "/st/f", D "/st/g"), 0);
    expect("own_file_out_of_sticky", rn(D "/st/g", D "/sg"), 0);
    expect("own_file_into_sticky", rn(D "/sg", D "/st/h"), 0);

    expect("cleanup_h", a_unlink(D "/st/h"), 0);
    expect("cleanup_st", a_rmdir(D "/st"), 0);
    expect("cleanup_rf", a_unlink(D "/rf"), 0);
    expect("cleanup_k", a_rmdir(D "/k"), 0);
    expect("cleanup_p", a_rmdir(D "/p"), 0);
    expect("cleanup_f0", a_unlink(D "/f0"), 0);
    expect("cleanup_ro_g", a_unlink(D "/ro/g"), 0);
    expect("cleanup_ro", a_rmdir(D "/ro"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
