/* link: the new name's directory needs write and search permission, the
 * old name's directories need search permission. A caller without
 * CAP_DAC_OVERRIDE gets EACCES; root passes these checks. The test
 * checks whichever applies to the caller. An existing new name is
 * reported before the permission problem, and the permission problem
 * before refusing a directory as the old name. */
#include "a_sys.h"

#define D "/tmp/a_link_permissions"

int test_main(void) {
    struct a_stat st;
    int root = syscall0(SYS_geteuid) == 0;
    i64 denied = root ? 0 : -EACCES;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_ro", a_mkdir(D "/ro", 0755), 0);
    expect("setup_nosearch", a_mkdir(D "/nosearch", 0755), 0);
    expect("setup_f", a_make_file(D "/f", "", 0644), 0);
    expect("setup_hidden", a_make_file(D "/nosearch/h", "", 0644), 0);
    expect("chmod_ro", syscall2(SYS_chmod, D "/ro", 0555), 0);
    expect("chmod_nosearch", syscall2(SYS_chmod, D "/nosearch", 0666), 0);
    expect("into_readonly_dir", syscall2(SYS_link, D "/f", D "/ro/x"), denied);
    expect("into_unsearchable_dir", syscall2(SYS_link, D "/f", D "/nosearch/x"), denied);
    expect("from_unsearchable_dir", syscall2(SYS_link, D "/nosearch/h", D "/y"), denied);
    /* errors found before the permission check still win */
    expect("readonly_dir_existing_name", syscall2(SYS_link, D "/f", D "/ro/."), -EEXIST);
    expect("readonly_dir_directory_source", syscall2(SYS_link, D, D "/ro/x"),
           root ? -EPERM : -EACCES);
    expect("chmod_back_ro", syscall2(SYS_chmod, D "/ro", 0755), 0);
    expect("chmod_back_nosearch", syscall2(SYS_chmod, D "/nosearch", 0755), 0);
    a_stat_path(D "/f", &st);
    expect("nlink", st.st_nlink, root ? 3 : 1);
    a_unlink(D "/ro/x");
    a_unlink(D "/nosearch/x");
    a_unlink(D "/y");
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_h", a_unlink(D "/nosearch/h"), 0);
    expect("cleanup_ro", a_rmdir(D "/ro"), 0);
    expect("cleanup_nosearch", a_rmdir(D "/nosearch"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
