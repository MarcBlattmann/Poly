/* The order in which fsconfig looks at its arguments. A negative fs_fd is
 * EINVAL before anything else (also with an unknown command); then the
 * command: 9 and above is EOPNOTSUPP even for a descriptor that is not open;
 * then the shape of the arguments (a key where none is allowed, a missing key
 * or value, a nonzero aux where it must be zero) is EINVAL, also for a
 * descriptor that is not open; only then the descriptor is looked up
 * (EBADF; a file that is no context is EINVAL) and only after that are key
 * and value read from memory (a bad pointer is EFAULT, and EBADF still wins
 * over it when the descriptor is closed). For FSCONFIG_SET_FD the aux
 * descriptor is looked up after the key was read and before the parameter is
 * known: a closed aux is EBADF for any key (also one the file system does not
 * have), an aux of -1 is EINVAL, an open aux for a string parameter or an
 * unknown key is EINVAL. fs_fd is an int: the upper 32 bits of the register
 * are ignored. Nothing here changes the context.
 * poly-diff: needs=root
 */
#include "p_mntapi.h"

#define BASE "/tmp/pk_fsconfig_order"

static void body(const char *base) {
    (void)base;
    i64 fc = pm_fsopen("tmpfs", FSOPEN_CLOEXEC);
    check("fsopen", fc >= 0, fc);
    const char *bad = (const char *)1;

    /* negative descriptor first, then the command */
    expect("neg1_flag", pm_fsconfig(-1, FSCONFIG_SET_FLAG, "ro", 0, 0), -EINVAL);
    expect("neg1_cmd99", pm_fsconfig(-1, 99, 0, 0, 0), -EINVAL);
    expect("neg1_create", pm_fsconfig(-1, FSCONFIG_CMD_CREATE, 0, 0, 0), -EINVAL);
    expect("neg2_create", pm_fsconfig(-2, FSCONFIG_CMD_CREATE, 0, 0, 0), -EINVAL);
    expect("neg100_string", pm_fsconfig(-100, FSCONFIG_SET_STRING, "size", "1m", 0), -EINVAL);
    expect("intmin_string", pm_fsconfig(-2147483647LL - 1, FSCONFIG_SET_STRING, "size", "1m", 0), -EINVAL);
    expect("neg1_set_fd_open_aux", pm_fsconfig(-1, FSCONFIG_SET_FD, "source", 0, 1), -EINVAL);
    expect("cmd9_closed_fd", pm_fsconfig(998, 9, 0, 0, 0), -EOPNOTSUPP);
    expect("cmd200_closed_fd", pm_fsconfig(998, 200, "k", "v", 7), -EOPNOTSUPP);
    expect("cmd9_open_fd", pm_fsconfig(fc, 9, 0, 0, 0), -EOPNOTSUPP);

    /* the shape of the arguments before the descriptor */
    expect("closed_flag_no_key", pm_fsconfig(998, FSCONFIG_SET_FLAG, 0, 0, 0), -EINVAL);
    expect("closed_flag_value", pm_fsconfig(998, FSCONFIG_SET_FLAG, "ro", "x", 0), -EINVAL);
    expect("closed_flag_aux", pm_fsconfig(998, FSCONFIG_SET_FLAG, "ro", 0, 5), -EINVAL);
    expect("closed_string_no_value", pm_fsconfig(998, FSCONFIG_SET_STRING, "size", 0, 0), -EINVAL);
    expect("closed_string_no_key", pm_fsconfig(998, FSCONFIG_SET_STRING, 0, "1m", 0), -EINVAL);
    expect("closed_string_aux", pm_fsconfig(998, FSCONFIG_SET_STRING, "size", "1m", 3), -EINVAL);
    expect("closed_create_key", pm_fsconfig(998, FSCONFIG_CMD_CREATE, "x", 0, 0), -EINVAL);
    expect("closed_reconf_value", pm_fsconfig(998, FSCONFIG_CMD_RECONFIGURE, 0, "x", 0), -EINVAL);
    expect("closed_excl_aux", pm_fsconfig(998, FSCONFIG_CMD_CREATE_EXCL, 0, 0, 9), -EINVAL);
    expect("closed_binary_zero", pm_fsconfig(998, FSCONFIG_SET_BINARY, "size", "1m", 0), -EINVAL);
    expect("closed_binary_toolong", pm_fsconfig(998, FSCONFIG_SET_BINARY, "size", "1m", 1048577), -EINVAL);
    expect("closed_path_no_key", pm_fsconfig(998, FSCONFIG_SET_PATH, 0, "/tmp", AT_FDCWD), -EINVAL);
    expect("closed_path_no_value", pm_fsconfig(998, FSCONFIG_SET_PATH, "source", 0, AT_FDCWD), -EINVAL);
    expect("closed_setfd_no_key", pm_fsconfig(998, FSCONFIG_SET_FD, 0, 0, 1), -EINVAL);
    expect("closed_setfd_value", pm_fsconfig(998, FSCONFIG_SET_FD, "source", "x", 1), -EINVAL);

    /* then the descriptor; the memory is read after it */
    expect("closed_string_ok_shape", pm_fsconfig(998, FSCONFIG_SET_STRING, "size", "1m", 0), -EBADF);
    expect("closed_create", pm_fsconfig(998, FSCONFIG_CMD_CREATE, 0, 0, 0), -EBADF);
    expect("closed_flag", pm_fsconfig(998, FSCONFIG_SET_FLAG, "ro", 0, 0), -EBADF);
    expect("closed_badptr_key", pm_fsconfig(998, FSCONFIG_SET_STRING, bad, "1", 0), -EBADF);
    expect("closed_badptr_value", pm_fsconfig(998, FSCONFIG_SET_STRING, "size", bad, 0), -EBADF);
    expect("open_badptr_key", pm_fsconfig(fc, FSCONFIG_SET_STRING, bad, "1", 0), -EFAULT);
    expect("open_badptr_value", pm_fsconfig(fc, FSCONFIG_SET_STRING, "size", bad, 0), -EFAULT);

    /* SET_FD: the aux descriptor */
    expect("setfd_closed_aux_size", pm_fsconfig(fc, FSCONFIG_SET_FD, "size", 0, 999), -EBADF);
    expect("setfd_closed_aux_unknown_key", pm_fsconfig(fc, FSCONFIG_SET_FD, "nosuchkey", 0, 999), -EBADF);
    expect("setfd_closed_aux_flag_key", pm_fsconfig(fc, FSCONFIG_SET_FD, "ro", 0, 999), -EBADF);
    expect("setfd_closed_aux_source", pm_fsconfig(fc, FSCONFIG_SET_FD, "source", 0, 999), -EBADF);
    expect("setfd_aux_minus1", pm_fsconfig(fc, FSCONFIG_SET_FD, "size", 0, -1), -EINVAL);
    expect("setfd_open_aux_size", pm_fsconfig(fc, FSCONFIG_SET_FD, "size", 0, 1), -EINVAL);
    expect("setfd_open_aux_unknown_key", pm_fsconfig(fc, FSCONFIG_SET_FD, "nosuchkey", 0, 1), -EINVAL);
    expect("setfd_open_aux_source", pm_fsconfig(fc, FSCONFIG_SET_FD, "source", 0, 1), -EINVAL);
    expect("setfd_badptr_key_closed_aux", pm_fsconfig(fc, FSCONFIG_SET_FD, bad, 0, 999), -EFAULT);
    expect("setfd_badptr_key_open_aux", pm_fsconfig(fc, FSCONFIG_SET_FD, bad, 0, 1), -EFAULT);

    /* the path commands: the type of the parameter before the path */
    expect("path_string_param", pm_fsconfig(fc, FSCONFIG_SET_PATH, "source", "/tmp", AT_FDCWD), -EINVAL);
    expect("path_closed_dirfd", pm_fsconfig(fc, FSCONFIG_SET_PATH, "size", "/tmp", 999), -EINVAL);
    expect("path_missing_target", pm_fsconfig(fc, FSCONFIG_SET_PATH, "source", "/nonexistent/x", AT_FDCWD), -EINVAL);
    expect("pathempty_string_param", pm_fsconfig(fc, FSCONFIG_SET_PATH_EMPTY, "source", "", 999), -EINVAL);
    expect("path_badptr_after_fd", pm_fsconfig(998, FSCONFIG_SET_PATH, "source", bad, AT_FDCWD), -EBADF);

    /* the high half of the register is ignored */
    expect("fd_high_bits_set", pm_fsconfig(fc + 4294967296LL, FSCONFIG_SET_STRING, "size", "1m", 0), 0);
    expect("fd_high_bits_closed", pm_fsconfig(998 + 4294967296LL, FSCONFIG_SET_STRING, "size", "1m", 0), -EBADF);
    expect("fd_high_bits_negative", pm_fsconfig(4294967295LL, FSCONFIG_SET_STRING, "size", "1m", 0), -EINVAL);

    /* the context took none of this except the one good size */
    expect("still_good", pm_fsconfig(fc, FSCONFIG_SET_STRING, "mode", "755", 0), 0);
    expect("create", pm_fsconfig(fc, FSCONFIG_CMD_CREATE, 0, 0, 0), 0);
    i64 m = pm_fsmount(fc, 0, 0);
    check("fsmount", m >= 0, m);
}

int test_main(void) { return pm_main(BASE, body); }
