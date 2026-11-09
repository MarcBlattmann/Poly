/* The life of a filesystem context as seen through fsconfig and fsmount.
 * From fsopen the context takes parameters; FSCONFIG_CMD_CREATE (or
 * CMD_CREATE_EXCL) builds the superblock. After that every fsconfig command
 * is EBUSY (a second create, parameters, reconfigure) until fsmount has
 * taken the mount, and fsmount itself needs the created state. After
 * fsmount the context is in the reconfigure state: parameters are accepted
 * again and FSCONFIG_CMD_RECONFIGURE applies them to the live mount;
 * create and a second fsmount are EBUSY. Before create, reconfigure is EBUSY
 * and fsmount is EINVAL. A rejected parameter leaves the context as it was;
 * a failed create or a failed reconfigure makes every later command EBUSY.
 * poly-diff: needs=root
 */
#include "p_mntapi.h"

#define BASE "/tmp/pk_fsconfig_states"

static void body(const char *base) {
    char p[128];
    i64 fc = pm_fsopen("tmpfs", FSOPEN_CLOEXEC);
    expect("reconfigure_before_create", pm_fsconfig(fc, FSCONFIG_CMD_RECONFIGURE, 0, 0, 0), -EBUSY);
    expect("fsmount_before_create", pm_fsmount(fc, 0, 0), -EINVAL);
    expect("params_still_ok", pm_fsconfig(fc, FSCONFIG_SET_STRING, "size", "1m", 0), 0);
    expect("create", pm_fsconfig(fc, FSCONFIG_CMD_CREATE, 0, 0, 0), 0);
    expect("create_twice", pm_fsconfig(fc, FSCONFIG_CMD_CREATE, 0, 0, 0), -EBUSY);
    expect("create_excl_after_create", pm_fsconfig(fc, FSCONFIG_CMD_CREATE_EXCL, 0, 0, 0), -EBUSY);
    expect("param_after_create", pm_fsconfig(fc, FSCONFIG_SET_STRING, "size", "2m", 0), -EBUSY);
    expect("flag_after_create", pm_fsconfig(fc, FSCONFIG_SET_FLAG, "ro", 0, 0), -EBUSY);
    expect("reconfigure_after_create", pm_fsconfig(fc, FSCONFIG_CMD_RECONFIGURE, 0, 0, 0), -EBUSY);
    i64 m = pm_fsmount(fc, FSMOUNT_CLOEXEC, 0);
    check("fsmount", m >= 0, m);
    expect("fsmount_twice", pm_fsmount(fc, 0, 0), -EBUSY);
    expect("create_after_fsmount", pm_fsconfig(fc, FSCONFIG_CMD_CREATE, 0, 0, 0), -EBUSY);
    expect("create_excl_after_fsmount", pm_fsconfig(fc, FSCONFIG_CMD_CREATE_EXCL, 0, 0, 0), -EBUSY);

    /* reconfigure state: the live mount changes */
    pm_path(p, base, "m");
    expect("mkdir_m", pm_mkdir(p), 0);
    expect("attach", pm_attach(m, p), 0);
    expect("blocks_before", pm_statfs_blocks(p), 256);
    expect("param_in_reconfigure_state", pm_fsconfig(fc, FSCONFIG_SET_STRING, "size", "2m", 0), 0);
    expect("reconfigure", pm_fsconfig(fc, FSCONFIG_CMD_RECONFIGURE, 0, 0, 0), 0);
    expect("blocks_after", pm_statfs_blocks(p), 512);
    expect("flag_ro", pm_fsconfig(fc, FSCONFIG_SET_FLAG, "ro", 0, 0), 0);
    expect("reconfigure_ro", pm_fsconfig(fc, FSCONFIG_CMD_RECONFIGURE, 0, 0, 0), 0);
    expect("now_read_only", pm_can_create(p), -EROFS);
    expect("flag_rw", pm_fsconfig(fc, FSCONFIG_SET_FLAG, "rw", 0, 0), 0);
    expect("reconfigure_rw", pm_fsconfig(fc, FSCONFIG_CMD_RECONFIGURE, 0, 0, 0), 0);
    expect("writable_again", pm_can_create(p), 0);
    expect("bad_param_keeps_state", pm_fsconfig(fc, FSCONFIG_SET_STRING, "bogus", "1", 0), -EINVAL);
    check("msg_unknown", pm_msg_is(fc, "e tmpfs: Unknown parameter 'bogus'\n"), 0);
    expect("reconfigure_still_ok", pm_fsconfig(fc, FSCONFIG_CMD_RECONFIGURE, 0, 0, 0), 0);
    /* a failed reconfigure ends the context: tmpfs refuses fewer inodes than are in use */
    expect("make_files", pm_touch(pm_path(p, base, "m/a")), 0);
    expect("make_files2", pm_touch(pm_path(p, base, "m/b")), 0);
    expect("one_inode", pm_fsconfig(fc, FSCONFIG_SET_STRING, "nr_inodes", "1", 0), 0);
    expect("reconfigure_fails", pm_fsconfig(fc, FSCONFIG_CMD_RECONFIGURE, 0, 0, 0), -EINVAL);
    check("msg_too_few_inodes", pm_msg_is(fc, "e tmpfs: Too few inodes for current use\n"), 0);
    expect("dead_param", pm_fsconfig(fc, FSCONFIG_SET_STRING, "size", "1m", 0), -EBUSY);
    expect("dead_reconfigure", pm_fsconfig(fc, FSCONFIG_CMD_RECONFIGURE, 0, 0, 0), -EBUSY);
    expect("mount_unharmed", pm_statfs_blocks(pm_path(p, base, "m")), 512);
    a_close(m);
    a_close(fc);

    /* a rejected parameter changes nothing; the same context goes on */
    fc = pm_fsopen("tmpfs", FSOPEN_CLOEXEC);
    expect("bad_param", pm_fsconfig(fc, FSCONFIG_SET_STRING, "mode", "xyz", 0), -EINVAL);
    expect("good_param", pm_fsconfig(fc, FSCONFIG_SET_STRING, "mode", "0700", 0), 0);
    expect("create_excl", pm_fsconfig(fc, FSCONFIG_CMD_CREATE_EXCL, 0, 0, 0), 0);
    m = pm_fsmount(fc, FSMOUNT_CLOEXEC, 0);
    check("fsmount_after_create_excl", m >= 0, m);
    a_close(m);
    a_close(fc);

    /* a failed create ends the context */
    fc = pm_fsopen("squashfs", FSOPEN_CLOEXEC);
    expect("create_without_source", pm_fsconfig(fc, FSCONFIG_CMD_CREATE, 0, 0, 0), -EINVAL);
    expect("failed_create_then_create", pm_fsconfig(fc, FSCONFIG_CMD_CREATE, 0, 0, 0), -EBUSY);
    expect("failed_create_then_param", pm_fsconfig(fc, FSCONFIG_SET_STRING, "source", "x", 0), -EBUSY);
    expect("failed_create_then_fsmount", pm_fsmount(fc, 0, 0), -EINVAL);
    a_close(fc);

    /* the mount fd and the context fd are different objects */
    fc = pm_fsopen("tmpfs", FSOPEN_CLOEXEC);
    pm_fsconfig(fc, FSCONFIG_CMD_CREATE, 0, 0, 0);
    m = pm_fsmount(fc, FSMOUNT_CLOEXEC, 0);
    expect("config_on_mount_fd", pm_fsconfig(m, FSCONFIG_SET_STRING, "size", "1m", 0), -EBADF);
    expect("fsmount_on_mount_fd", pm_fsmount(m, 0, 0), -EBADF);
}

int test_main(void) { return pm_main(BASE, body); }
