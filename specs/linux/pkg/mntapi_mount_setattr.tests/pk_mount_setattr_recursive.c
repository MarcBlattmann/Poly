/* AT_RECURSIVE (0x8000) applies the change to the mount at path and every
 * mount below it; without it only the top mount changes. Making a mount
 * read-only fails with EBUSY while a file on it is open for writing (and
 * then changes nothing, also not for the mounts that could have been
 * changed). The change is per mount: a bind mount of the same filesystem
 * elsewhere keeps its own options. Removing the flag from a tree also works
 * recursively.
 * poly-diff: needs=root
 */
#include "p_mntapi.h"

#define BASE "/tmp/pk_mount_setattr_recursive"

static i64 setattr(const char *path, u64 flags, u64 set, u64 clr) {
    struct pm_mount_attr a = {set, clr, 0, 0};
    return pm_mount_setattr(AT_FDCWD, path, flags, &a, sizeof a);
}

static void body(const char *base) {
    char t[128], sub[160], deep[200], bnd[128], q[240];
    pm_path(t, base, "t");
    expect("mkdir_t", pm_mkdir(t), 0);
    expect("mount_t", pm_mount("t", t, "tmpfs", 0, "size=1m"), 0);
    expect("mkdir_sub", pm_mkdir(pm_path(sub, t, "sub")), 0);
    expect("mount_sub", pm_mount("s", sub, "tmpfs", 0, "size=2m"), 0);
    expect("mkdir_deep", pm_mkdir(pm_path(deep, sub, "deep")), 0);
    expect("mount_deep", pm_mount("d", deep, "tmpfs", 0, "size=3m"), 0);
    pm_path(bnd, base, "bnd");
    expect("mkdir_bnd", pm_mkdir(bnd), 0);
    expect("bind_sub", pm_mount(sub, bnd, 0, MS_BIND, 0), 0);

    /* top only */
    expect("top_only", setattr(t, 0, MOUNT_ATTR_NOSUID, 0), 0);
    expect("top_nosuid", pm_mntflags(t) & ST_NOSUID, ST_NOSUID);
    expect("sub_unchanged", pm_mntflags(sub) & ST_NOSUID, 0);
    expect("deep_unchanged", pm_mntflags(deep) & ST_NOSUID, 0);

    /* recursive */
    expect("recursive", setattr(t, AT_RECURSIVE, MOUNT_ATTR_NODEV | MOUNT_ATTR_NOSUID, 0), 0);
    expect("top_nodev", pm_mntflags(t) & (ST_NODEV | ST_NOSUID), ST_NODEV | ST_NOSUID);
    expect("sub_nodev", pm_mntflags(sub) & (ST_NODEV | ST_NOSUID), ST_NODEV | ST_NOSUID);
    expect("deep_nodev", pm_mntflags(deep) & (ST_NODEV | ST_NOSUID), ST_NODEV | ST_NOSUID);
    expect("bind_of_sub_not_in_tree", pm_mntflags(bnd) & (ST_NODEV | ST_NOSUID), 0);

    /* recursive read-only blocked by an open writer */
    i64 w = a_open(pm_path(q, deep, "open-for-write"), O_WRONLY | O_CREAT, 0644);
    check("open_writer", w >= 0, w);
    expect("rdonly_busy", setattr(t, AT_RECURSIVE, MOUNT_ATTR_RDONLY, 0), -EBUSY);
    expect("top_still_rw", pm_can_create(t), 0);
    expect("sub_still_rw", pm_can_create(sub), 0);
    expect("flags_unchanged", pm_mntflags(t) & ST_RDONLY, 0);
    expect("rdonly_deep_busy", setattr(deep, 0, MOUNT_ATTR_RDONLY, 0), -EBUSY);
    expect("rdonly_sub_alone_ok", setattr(sub, 0, MOUNT_ATTR_RDONLY, 0), 0);
    expect("sub_ro", pm_can_create(sub), -EROFS);
    expect("deep_still_rw", pm_can_create(deep), 0);
    expect("bind_of_sub_is_separate", pm_can_create(bnd), 0);
    a_close(w);
    expect("clear_sub", setattr(sub, 0, 0, MOUNT_ATTR_RDONLY), 0);

    /* without the writer it works, in the whole tree */
    expect("rdonly_recursive", setattr(t, AT_RECURSIVE, MOUNT_ATTR_RDONLY, 0), 0);
    expect("ro_top", pm_can_create(t), -EROFS);
    expect("ro_sub", pm_can_create(sub), -EROFS);
    expect("ro_deep", pm_can_create(deep), -EROFS);
    expect("rw_bind", pm_can_create(bnd), 0);
    expect("clear_recursive", setattr(t, AT_RECURSIVE, 0, MOUNT_ATTR_RDONLY | MOUNT_ATTR_NODEV | MOUNT_ATTR_NOSUID), 0);
    expect("rw_top", pm_can_create(t), 0);
    expect("rw_sub", pm_can_create(sub), 0);
    expect("rw_deep", pm_can_create(deep), 0);
    expect("flags_clean_deep", pm_mntflags(deep), ST_RELATIME);
    expect("flags_clean_sub", pm_mntflags(sub), ST_RELATIME);
}

int test_main(void) { return pm_main(BASE, body); }
