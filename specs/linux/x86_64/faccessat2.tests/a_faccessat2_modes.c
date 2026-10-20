/* faccessat2 with no flags agrees with faccessat on every mode and file
 * type; mode bits above X_OK are refused. */
#include "a_sys.h"

#define D "/tmp/a_faccessat2_modes"

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0700), 0);
    const char *files[] = {D "/m000", D "/m100", D "/m010", D "/m001", D "/m644", D "/m755"};
    const i64 modes[] = {0000, 0100, 0010, 0001, 0644, 0755};
    int ok = 1;
    for (int i = 0; i < 6; i++) ok &= a_make_file(files[i], "", modes[i]) == 0;
    check("setup_files", ok, 0);
    int agree = 1;
    for (int i = 0; i < 6; i++)
        for (i64 m = 0; m < 8; m++)
            agree &= syscall4(SYS_faccessat2, AT_FDCWD, files[i], m, 0) == syscall3(SYS_faccessat, AT_FDCWD, files[i], m);
    check("agrees_with_faccessat", agree, 0);
    expect("m000_exec_eacces", syscall4(SYS_faccessat2, AT_FDCWD, D "/m000", X_OK, 0), -EACCES);
    expect("m010_exec_ok", syscall4(SYS_faccessat2, AT_FDCWD, D "/m010", X_OK, 0), 0);
    expect("m644_exec_eacces", syscall4(SYS_faccessat2, AT_FDCWD, D "/m644", X_OK, 0), -EACCES);
    expect("m755_all", syscall4(SYS_faccessat2, AT_FDCWD, D "/m755", R_OK | W_OK | X_OK, 0), 0);
    int bad = 1;
    for (i64 m = 8; m < 64; m += 8) bad &= syscall4(SYS_faccessat2, AT_FDCWD, D "/m755", m, 0) == -EINVAL;
    check("high_mode_bits_einval", bad, 0);
    expect("dir_search", syscall4(SYS_faccessat2, AT_FDCWD, D, X_OK, 0), 0);
    for (int i = 0; i < 6; i++) ok &= a_unlink(files[i]) == 0;
    check("cleanup_files", ok, 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
