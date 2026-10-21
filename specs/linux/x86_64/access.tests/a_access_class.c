/* access: the owner of a file is checked against the owner bits alone, even
 * when the group or other bits would allow more (mode 0077: the owner is
 * denied everything). The caller's own group and "other" are not consulted
 * for the owner. Root passes R_OK/W_OK, and X_OK as an execute bit is set. */
#include "a_sys.h"

#define D "/tmp/a_access_class"

int test_main(void) {
    int root = syscall0(SYS_getuid) == 0;
    i64 den = root ? 0 : -EACCES;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/g", "", 0600), 0);
    expect("setup_chmod_0077", syscall2(SYS_chmod, D "/g", 0077), 0);
    expect("setup_file2", a_make_file(D "/o", "", 0600), 0);
    expect("setup_chmod_0407", syscall2(SYS_chmod, D "/o", 0407), 0);
    expect("owner_0077_r", syscall2(SYS_access, D "/g", R_OK), den);
    expect("owner_0077_w", syscall2(SYS_access, D "/g", W_OK), den);
    expect("owner_0077_x", syscall2(SYS_access, D "/g", X_OK), den);
    expect("owner_0077_f", syscall2(SYS_access, D "/g", F_OK), 0);
    expect("owner_0407_r", syscall2(SYS_access, D "/o", R_OK), 0);
    expect("owner_0407_w", syscall2(SYS_access, D "/o", W_OK), den);
    expect("owner_0407_x", syscall2(SYS_access, D "/o", X_OK), den);
    expect("cleanup_g", a_unlink(D "/g"), 0);
    expect("cleanup_o", a_unlink(D "/o"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
