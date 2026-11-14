/* move_mount with MOVE_MOUNT_SET_GROUP (0x100): the target mount joins the
 * propagation group of the source; nothing is moved. Both must be tops of
 * mounts of the same file system (two mounts that show different directories
 * of it are fine, the same mount twice is not). The source must be a shared
 * mount or a slave and the target a private mount: the target then becomes a
 * peer of a shared source (mountinfo shows "shared:N" with the source's N,
 * and mounts made below one appear below the other), or a slave of the same
 * master as a slave source ("master:N"). A private source (EINVAL), a target
 * that is shared already, in the same group or another (EINVAL), the same
 * mount for both, a path that is not the top of a mount, mounts of different
 * file systems: EINVAL, and nothing changes. The flag may come with others
 * (MOVE_MOUNT_F_SYMLINKS).
 * poly-diff: needs=root
 * poly-diff: syscalls=off (group numbers are not the same on every run)
 */
#include "p_mntapi.h"

#define BASE "/tmp/pk_move_group"
#define P(x) BASE x

/* /proc/self/mountinfo as text */
static char mi_text[65536];

static void load_mountinfo(void) {
    size_t got = 0;
    i64 fd = a_open("/proc/self/mountinfo", O_RDONLY, 0);
    if (fd >= 0) {
        for (;;) {
            i64 n = a_read(fd, mi_text + got, sizeof mi_text - 1 - got);
            if (n <= 0) break;
            got += (size_t)n;
        }
        a_close(fd);
    }
    mi_text[got] = 0;
}

/* The digits after `prefix` ("shared:" or "master:") in the optional fields
 * of the mount at `mp`; "" when the tag is missing. A line reads: id parent
 * major:minor root mountpoint options [optional fields] - fstype source ... */
static void tag(const char *mp, const char *prefix, char *out) {
    out[0] = 0;
    load_mountinfo();
    size_t plen = str_len(prefix);
    size_t ml = str_len(mp);
    const char *line = mi_text;
    while (*line) {
        const char *end = line;
        while (*end && *end != 10) end++;
        int field = 0;
        int match = 0;
        const char *s = line;
        while (s < end) {
            const char *f = s;
            while (s < end && *s != ' ') s++;
            size_t len = (size_t)(s - f);
            if (field == 4) {
                match = len == ml;
                for (size_t k = 0; match && k < ml; k++) match = f[k] == mp[k];
                if (!match) break;
            } else if (field >= 6 && match) {
                if (len == 1 && f[0] == '-') break;
                int is_tag = len > plen;
                for (size_t k = 0; is_tag && k < plen; k++) is_tag = f[k] == prefix[k];
                if (is_tag) {
                    size_t o = 0;
                    for (size_t k = plen; k < len && o < 15; k++) out[o++] = f[k];
                    out[o] = 0;
                    return;
                }
            }
            if (s < end) s++;
            field++;
        }
        line = *end ? end + 1 : end;
    }
}

static int has(const char *mp, const char *prefix) {
    char t[16];
    tag(mp, prefix, t);
    return t[0] != 0;
}

static int same_group(const char *mp1, const char *p1, const char *mp2, const char *p2) {
    char a[16], b[16];
    tag(mp1, p1, a);
    tag(mp2, p2, b);
    return a[0] != 0 && str_eq(a, b);
}

static i64 sg(const char *from, const char *to, u64 extra) {
    return pm_move_mount(AT_FDCWD, from, AT_FDCWD, to, MOVE_MOUNT_SET_GROUP | extra);
}

static i64 bind(const char *from, const char *to) { return pm_mount(from, to, 0, MS_BIND, 0); }
static i64 make(const char *p, u64 flag) { return pm_mount(0, p, 0, flag, 0); }

static void body(const char *base) {
    (void)base;
    const char *dirs[] = {P("/a"), P("/b"), P("/c"), P("/d"), P("/e"), P("/f"), P("/g"), P("/h"), P("/i"), 0};
    for (int i = 0; dirs[i]; i++) a_mkdir(dirs[i], 0755);
    expect("mount_a", pm_mount("a", P("/a"), "tmpfs", 0, "size=1m"), 0);
    expect("mkdir_sub", a_mkdir(P("/a/sub"), 0755), 0);
    expect("bind_b", bind(P("/a"), P("/b")), 0);

    /* a private source */
    expect("private_to_private", sg(P("/a"), P("/b"), 0), -EINVAL);
    check("b_still_private", !has(P("/b"), "shared:") && !has(P("/b"), "master:"), 0);

    /* shared source, private target */
    expect("make_a_shared", make(P("/a"), MS_SHARED), 0);
    expect("shared_to_private", sg(P("/a"), P("/b"), 0), 0);
    check("a_is_shared", has(P("/a"), "shared:"), 0);
    check("b_joined_a", same_group(P("/a"), "shared:", P("/b"), "shared:"), 0);
    expect("mount_below_a", pm_mount("in", P("/a/sub"), "tmpfs", 0, 0), 0);
    expect("file_below_a", a_make_file(P("/a/sub/f"), "f", 0644), 0);
    expect("peer_sees_mount_below", pm_exists(P("/b/sub/f")), 0);
    expect("umount_below_a", pm_umount2(P("/a/sub"), 0), 0);
    expect("peer_lost_it_too", pm_exists(P("/b/sub/f")), -ENOENT);

    /* target shared already */
    expect("shared_to_peer", sg(P("/a"), P("/b"), 0), -EINVAL);
    expect("bind_c", bind(P("/a"), P("/c")), 0);
    expect("c_private", make(P("/c"), MS_PRIVATE), 0);
    expect("c_shared_new_group", make(P("/c"), MS_SHARED), 0);
    check("c_other_group", has(P("/c"), "shared:") && !same_group(P("/a"), "shared:", P("/c"), "shared:"), 0);
    expect("shared_to_shared_other_group", sg(P("/a"), P("/c"), 0), -EINVAL);
    expect("same_mount", sg(P("/a"), P("/a"), 0), -EINVAL);

    /* a private source for a shared target */
    expect("bind_e", bind(P("/a"), P("/e")), 0);
    expect("e_private", make(P("/e"), MS_PRIVATE), 0);
    expect("private_source_shared_target", sg(P("/e"), P("/c"), 0), -EINVAL);

    /* what must be the top of a mount */
    expect("source_not_a_top", sg(P("/a/sub"), P("/e"), 0), -EINVAL);
    expect("target_not_a_top", sg(P("/a"), P("/a/sub"), 0), -EINVAL);

    /* another file system */
    expect("mount_d", pm_mount("d", P("/d"), "tmpfs", 0, "size=1m"), 0);
    expect("other_fs_target", sg(P("/a"), P("/d"), 0), -EINVAL);
    expect("make_d_shared", make(P("/d"), MS_SHARED), 0);
    expect("other_fs_source", sg(P("/d"), P("/e"), 0), -EINVAL);
    check("e_still_private", !has(P("/e"), "shared:") && !has(P("/e"), "master:"), 0);

    /* a slave source: the target becomes a slave of the same master */
    expect("bind_f", bind(P("/a"), P("/f")), 0);
    expect("f_slave", make(P("/f"), MS_SLAVE), 0);
    check("f_has_master", has(P("/f"), "master:") && !has(P("/f"), "shared:"), 0);
    expect("bind_g", bind(P("/a"), P("/g")), 0);
    expect("g_private", make(P("/g"), MS_PRIVATE), 0);
    expect("slave_to_private", sg(P("/f"), P("/g"), 0), 0);
    check("g_slave_of_same_master", same_group(P("/f"), "master:", P("/g"), "master:"), 0);
    check("g_not_shared", !has(P("/g"), "shared:"), 0);
    expect("slave_to_slave", sg(P("/f"), P("/g"), 0), -EINVAL);

    /* another directory of the same file system, and another flag with it */
    expect("bind_h", bind(P("/a/sub"), P("/h")), 0);
    expect("h_private", make(P("/h"), MS_PRIVATE), 0);
    expect("shared_to_subdir_mount", sg(P("/a"), P("/h"), 0), 0);
    check("h_joined_a", same_group(P("/a"), "shared:", P("/h"), "shared:"), 0);
    expect("bind_i", bind(P("/a"), P("/i")), 0);
    expect("i_private", make(P("/i"), MS_PRIVATE), 0);
    expect("with_symlinks_flag", sg(P("/a"), P("/i"), MOVE_MOUNT_F_SYMLINKS), 0);
    check("i_joined_a", same_group(P("/a"), "shared:", P("/i"), "shared:"), 0);
    expect("unknown_bit", sg(P("/a"), P("/i"), 0x1000), -EINVAL);
}

int test_main(void) { return pm_main(BASE, body); }
