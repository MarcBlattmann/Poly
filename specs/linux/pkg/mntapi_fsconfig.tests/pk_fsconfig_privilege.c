/* Who may do what with a context descriptor, and what a refusal leaves
 * behind. A descriptor handed to a process without CAP_SYS_ADMIN (uid 65534,
 * no capabilities) still takes parameters, valid or not (EINVAL for a bad one,
 * with its message), before and after the file system was created. Its
 * CMD_CREATE and CMD_CREATE_EXCL are EPERM; that refusal changes nothing: the
 * context is still taking parameters and a privileged CMD_CREATE succeeds.
 * The state is checked before the privilege (a second CMD_CREATE, a
 * CMD_RECONFIGURE before create: EBUSY), the shape of the arguments before
 * the state (CREATE with a key: EINVAL). fsmount of the created context by
 * such a process is EPERM and leaves the context usable for fsmount by root.
 * In the reconfigure state (after fsmount, and for a descriptor from fspick)
 * parameters are accepted from it but CMD_RECONFIGURE is EPERM, and this
 * refusal does end the context: every later command is EBUSY, fsmount too.
 * fsopen and fspick themselves are EPERM for it.
 * poly-diff: needs=root
 */
#include "p_mntapi.h"

#define BASE "/tmp/pk_fsconfig_priv"

static i64 g_fc, g_cmd, g_aux;
static const char *g_key, *g_val;
static i64 nb_cfg(void *arg) {
    (void)arg;
    return pm_fsconfig(g_fc, g_cmd, g_key, g_val, g_aux);
}
static i64 nb_fsmount(void *arg) {
    (void)arg;
    return pm_fsmount(g_fc, 0, 0);
}
static i64 nb_fsopen(void *arg) {
    (void)arg;
    return pm_fsopen("tmpfs", 0);
}
static i64 nb_fspick(void *arg) {
    return pm_fspick(AT_FDCWD, (const char *)arg, 0);
}
static i64 as(i64 fc, u64 cmd, const char *k, const char *v, i64 aux) {
    g_fc = fc;
    g_cmd = cmd;
    g_key = k;
    g_val = v;
    g_aux = aux;
    return pm_as_nobody(nb_cfg, 0);
}

static void body(const char *base) {
    i64 fc = pm_fsopen("tmpfs", FSOPEN_CLOEXEC);
    check("fsopen", fc >= 0, fc);
    expect("root_size", pm_fsconfig(fc, FSCONFIG_SET_STRING, "size", "1m", 0), 0);

    /* before create */
    expect("nobody_set", as(fc, FSCONFIG_SET_STRING, "mode", "755", 0), 0);
    expect("nobody_flag", as(fc, FSCONFIG_SET_FLAG, "ro", 0, 0), 0);
    expect("nobody_bad_param", as(fc, FSCONFIG_SET_STRING, "nosuchkey", "1", 0), -EINVAL);
    expect("nobody_bad_value", as(fc, FSCONFIG_SET_STRING, "size", "abc", 0), -EINVAL);
    expect("nobody_reconf_before_create", as(fc, FSCONFIG_CMD_RECONFIGURE, 0, 0, 0), -EBUSY);
    expect("nobody_create_with_key", as(fc, FSCONFIG_CMD_CREATE, "x", 0, 0), -EINVAL);
    expect("nobody_create", as(fc, FSCONFIG_CMD_CREATE, 0, 0, 0), -EPERM);
    expect("nobody_create_excl", as(fc, FSCONFIG_CMD_CREATE_EXCL, 0, 0, 0), -EPERM);
    expect("nobody_create_again", as(fc, FSCONFIG_CMD_CREATE, 0, 0, 0), -EPERM);
    expect("nobody_set_after_eperm", as(fc, FSCONFIG_SET_STRING, "uid", "5", 0), 0);
    expect("root_set_after_eperm", pm_fsconfig(fc, FSCONFIG_SET_STRING, "gid", "6", 0), 0);
    expect("root_create", pm_fsconfig(fc, FSCONFIG_CMD_CREATE, 0, 0, 0), 0);

    /* created */
    expect("nobody_set_created", as(fc, FSCONFIG_SET_STRING, "size", "2m", 0), -EBUSY);
    expect("nobody_create_created", as(fc, FSCONFIG_CMD_CREATE, 0, 0, 0), -EBUSY);
    expect("nobody_reconf_created", as(fc, FSCONFIG_CMD_RECONFIGURE, 0, 0, 0), -EBUSY);
    g_fc = fc;
    expect("nobody_fsmount", pm_as_nobody(nb_fsmount, 0), -EPERM);
    i64 mfd = pm_fsmount(fc, FSMOUNT_CLOEXEC, 0);
    check("root_fsmount_after_eperm", mfd >= 0, mfd);

    /* reconfigure state */
    expect("nobody_set_in_reconf", as(fc, FSCONFIG_SET_STRING, "size", "3m", 0), 0);
    expect("nobody_bad_in_reconf", as(fc, FSCONFIG_SET_STRING, "nosuchkey", "1", 0), -EINVAL);
    expect("nobody_create_in_reconf", as(fc, FSCONFIG_CMD_CREATE, 0, 0, 0), -EBUSY);
    expect("nobody_reconf", as(fc, FSCONFIG_CMD_RECONFIGURE, 0, 0, 0), -EPERM);
    expect("root_reconf_after_eperm", pm_fsconfig(fc, FSCONFIG_CMD_RECONFIGURE, 0, 0, 0), -EBUSY);
    expect("root_set_after_eperm_reconf", pm_fsconfig(fc, FSCONFIG_SET_STRING, "size", "4m", 0), -EBUSY);
    expect("root_fsmount_after_dead", pm_fsmount(fc, 0, 0), -EBUSY);

    /* a descriptor from fspick */
    i64 pc = pm_fspick(AT_FDCWD, base, FSPICK_CLOEXEC);
    check("root_fspick", pc >= 0, pc);
    expect("nobody_set_picked", as(pc, FSCONFIG_SET_STRING, "size", "2m", 0), 0);
    expect("nobody_create_picked", as(pc, FSCONFIG_CMD_CREATE, 0, 0, 0), -EBUSY);
    expect("nobody_reconf_picked", as(pc, FSCONFIG_CMD_RECONFIGURE, 0, 0, 0), -EPERM);
    expect("root_reconf_picked_dead", pm_fsconfig(pc, FSCONFIG_CMD_RECONFIGURE, 0, 0, 0), -EBUSY);
    i64 pc2 = pm_fspick(AT_FDCWD, base, FSPICK_CLOEXEC);
    check("root_fspick_again", pc2 >= 0, pc2);
    expect("nobody_set_picked2", as(pc2, FSCONFIG_SET_STRING, "size", "3m", 0), 0);
    expect("root_reconf_picked2", pm_fsconfig(pc2, FSCONFIG_CMD_RECONFIGURE, 0, 0, 0), 0);

    /* making contexts */
    expect("nobody_fsopen", pm_as_nobody(nb_fsopen, 0), -EPERM);
    expect("nobody_fspick", pm_as_nobody(nb_fspick, (void *)base), -EPERM);
}

int test_main(void) { return pm_main(BASE, body); }
