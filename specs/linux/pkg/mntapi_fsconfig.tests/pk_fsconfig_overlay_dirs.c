/* The directories of an overlay are looked up when they are set with
 * FSCONFIG_SET_STRING, not at create time. lowerdir (every ':' separated
 * part, also for lowerdir+), upperdir and workdir that do not exist are
 * ENOENT from that very call, with no message, and the parameter is not
 * taken. One that exists but is no directory is EINVAL with the message
 * "e overlay: PATH is not a directory" (PATH as it was given). An empty
 * upperdir is EINVAL ("Bad value for 'upperdir'"), a lowerdir list with an
 * empty part at its end EINVAL, a directory given with FSCONFIG_SET_PATH
 * EINVAL ("Bad value for 'lowerdir'"). A good value is accepted whether or
 * not the others are there yet; the same directory as upperdir and workdir is
 * accepted when set and refused at create (EINVAL).
 * poly-diff: needs=root
 */
#include "p_mntapi.h"

#define BASE "/tmp/pk_fsconfig_ovdirs"

/* the next message of the context, without its newline; "" when none */
static const char *next_msg(i64 fc, char *buf, size_t cap) {
    i64 n = syscall3(SYS_read, fc, buf, cap - 1);
    if (n < 0) {
        buf[0] = 0;
        return buf;
    }
    if (n > 0 && buf[n - 1] == '\n') n--;
    buf[n] = 0;
    return buf;
}

static void body(const char *base) {
    char d[128], d2[128], f[128], two[300], want[300], m[300];
    pm_path(d, base, "/dir");
    pm_path(d2, base, "/dir2");
    pm_path(f, base, "/file");
    a_mkdir(d, 0755);
    a_mkdir(d2, 0755);
    a_make_file(f, "x", 0644);
    i64 fc = pm_fsopen("overlay", FSOPEN_CLOEXEC);
    check("fsopen", fc >= 0, fc);

    expect("lower_missing", pm_fsconfig(fc, FSCONFIG_SET_STRING, "lowerdir", "/nonexistent_dir", 0), -ENOENT);
    expect("upper_missing", pm_fsconfig(fc, FSCONFIG_SET_STRING, "upperdir", "/nonexistent_dir", 0), -ENOENT);
    expect("work_missing", pm_fsconfig(fc, FSCONFIG_SET_STRING, "workdir", "/nonexistent_dir", 0), -ENOENT);
    expect("lower_relative_missing", pm_fsconfig(fc, FSCONFIG_SET_STRING, "lowerdir", "no_such_relative", 0), -ENOENT);
    expect("lowerdir_plus_missing", pm_fsconfig(fc, FSCONFIG_SET_STRING, "lowerdir+", "/nonexistent_dir", 0), -ENOENT);
    str_cat2(two, d, ":/nonexistent_dir");
    expect("lower_second_missing", pm_fsconfig(fc, FSCONFIG_SET_STRING, "lowerdir", two, 0), -ENOENT);
    str_cat2(two, "/nonexistent_dir:", d);
    expect("lower_first_missing", pm_fsconfig(fc, FSCONFIG_SET_STRING, "lowerdir", two, 0), -ENOENT);
    expect("no_message_for_missing", syscall3(SYS_read, fc, m, sizeof m), -ENODATA);

    /* exists, but no directory */
    expect("lower_file", pm_fsconfig(fc, FSCONFIG_SET_STRING, "lowerdir", f, 0), -EINVAL);
    str_cat2(want, "e overlay: ", f);
    str_cat2(want + str_len(want), "", " is not a directory");
    check("lower_file_message", str_eq(next_msg(fc, m, sizeof m), want), 0);
    expect("upper_file", pm_fsconfig(fc, FSCONFIG_SET_STRING, "upperdir", f, 0), -EINVAL);
    check("upper_file_message", str_eq(next_msg(fc, m, sizeof m), want), 0);
    expect("work_file", pm_fsconfig(fc, FSCONFIG_SET_STRING, "workdir", f, 0), -EINVAL);
    check("work_file_message", str_eq(next_msg(fc, m, sizeof m), want), 0);
    str_cat2(two, d, ":");
    str_cat2(two + str_len(two), f, "");
    expect("lower_second_file", pm_fsconfig(fc, FSCONFIG_SET_STRING, "lowerdir", two, 0), -EINVAL);
    check("lower_second_file_message", str_eq(next_msg(fc, m, sizeof m), want), 0);

    /* other bad values */
    expect("upper_empty", pm_fsconfig(fc, FSCONFIG_SET_STRING, "upperdir", "", 0), -EINVAL);
    check("upper_empty_message", str_eq(next_msg(fc, m, sizeof m), "e overlay: Bad value for 'upperdir'"), 0);
    str_cat2(two, d, ":");
    expect("lower_trailing_colon", pm_fsconfig(fc, FSCONFIG_SET_STRING, "lowerdir", two, 0), -EINVAL);
    expect("lower_as_path", pm_fsconfig(fc, FSCONFIG_SET_PATH, "lowerdir", d, AT_FDCWD), -EINVAL);
    check("lower_as_path_message", str_eq(next_msg(fc, m, sizeof m), "e overlay: Bad value for 'lowerdir'"), 0);
    expect("upper_as_path", pm_fsconfig(fc, FSCONFIG_SET_PATH, "upperdir", d, AT_FDCWD), -EINVAL);
    check("upper_as_path_message", str_eq(next_msg(fc, m, sizeof m), "e overlay: Bad value for 'upperdir'"), 0);
    expect("nothing_else_queued", syscall3(SYS_read, fc, m, sizeof m), -ENODATA);

    /* none of this was taken: the good values now work */
    expect("lower_ok", pm_fsconfig(fc, FSCONFIG_SET_STRING, "lowerdir", d, 0), 0);
    expect("upper_ok", pm_fsconfig(fc, FSCONFIG_SET_STRING, "upperdir", d2, 0), 0);
    expect("work_same_as_upper", pm_fsconfig(fc, FSCONFIG_SET_STRING, "workdir", d2, 0), 0);
    expect("create_same_upper_work", pm_fsconfig(fc, FSCONFIG_CMD_CREATE, 0, 0, 0), -EINVAL);
    expect("context_dead", pm_fsconfig(fc, FSCONFIG_SET_STRING, "lowerdir", d, 0), -EBUSY);

    /* a failed set is not a set: with only the missing lowerdir there is no lower layer */
    fc = pm_fsopen("overlay", FSOPEN_CLOEXEC);
    expect("lower_missing_again", pm_fsconfig(fc, FSCONFIG_SET_STRING, "lowerdir", "/nonexistent_dir", 0), -ENOENT);
    expect("upper_ok_2", pm_fsconfig(fc, FSCONFIG_SET_STRING, "upperdir", d2, 0), 0);
    expect("create_without_lower", pm_fsconfig(fc, FSCONFIG_CMD_CREATE, 0, 0, 0), -EINVAL);

    /* a working overlay out of values set in any order */
    char w[128];
    pm_path(w, base, "/work");
    a_mkdir(w, 0755);
    fc = pm_fsopen("overlay", FSOPEN_CLOEXEC);
    expect("work_first", pm_fsconfig(fc, FSCONFIG_SET_STRING, "workdir", w, 0), 0);
    expect("upper_second", pm_fsconfig(fc, FSCONFIG_SET_STRING, "upperdir", d2, 0), 0);
    expect("lower_third", pm_fsconfig(fc, FSCONFIG_SET_STRING, "lowerdir", d, 0), 0);
    expect("create_ok", pm_fsconfig(fc, FSCONFIG_CMD_CREATE, 0, 0, 0), 0);
    i64 mfd = pm_fsmount(fc, FSMOUNT_CLOEXEC, 0);
    check("fsmount", mfd >= 0, mfd);
}

int test_main(void) { return pm_main(BASE, body); }
