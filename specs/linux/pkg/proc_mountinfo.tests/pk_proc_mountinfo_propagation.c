/* The optional fields of /proc/self/mountinfo (between the options and the
 * "-" separator) tell the propagation state: a private mount has none; a
 * shared one has "shared:N"; a slave "master:N" (N is the peer group it
 * receives from); a slave that is shared again "shared:M master:N" (shared
 * first); an unbindable mount "unbindable". Mounts that are peers carry the
 * same N; a slave's master:N equals the shared:N of the mounts it follows.
 * The group numbers themselves vary and are never printed here. The number
 * of fields of a line grows with the optional fields (10 without any).
 * poly-diff: needs=root (mount needs CAP_SYS_ADMIN)
 * poly-diff: syscalls=off (the mount runs in a child in a mount namespace of its own)
 */
#include "p_mnt.h"

#define B "/tmp/pk_mip"
#define T B "/t"

/* the digits of "shared:N" / "master:N" inside an optional string, or "" */
static const char *group_of(const struct pm_mi *m, const char *prefix, char *out) {
    out[0] = 0;
    int at = pm_find(m->optional, prefix);
    if (at < 0) return out;
    const char *p = m->optional + at + str_len(prefix);
    size_t n = 0;
    while (p[n] >= '0' && p[n] <= '9' && n < 15) {
        out[n] = p[n];
        n++;
    }
    out[n] = 0;
    return out;
}

static struct pm_mi get(const char *mp) {
    struct pm_mi m;
    memset(&m, 0, sizeof m);
    pm_mi_load("/proc/self/mountinfo");
    pm_mi_get(mp, -1, &m);
    return m;
}

static int body(void) {
    pm_mkdirs(T);
    expect("mount_t", pm_mount("pkp", T, "tmpfs", 0, 0), 0);
    pm_mkdirs(T "/peer");
    pm_mkdirs(T "/slave");
    pm_mkdirs(T "/priv");
    pm_mkdirs(T "/unb");
    pm_mkdirs(T "/both");
    pm_mkdirs(B "/outside");
    struct pm_mi m;
    char g1[16], g2[16], g3[16];

    m = get(T);
    expect("private_no_optional", m.optional[0], 0);
    expect("private_ten_fields", m.nfields, 10);

    expect("make_shared", pm_mount(0, T, 0, MS_SHARED, 0), 0);
    m = get(T);
    expect("shared_one_optional", pm_group(m.optional, "shared:") != 0, 1);
    expect("shared_eleven_fields", m.nfields, 11);
    expect("shared_group_nonempty", group_of(&m, "shared:", g1)[0] != 0, 1);

    /* a peer */
    expect("bind_peer", pm_mount(T, B "/outside", 0, MS_BIND, 0), 0);
    struct pm_mi peer = get(B "/outside");
    group_of(&peer, "shared:", g2);
    expect("peer_is_shared", pm_group(peer.optional, "shared:") != 0, 1);
    expect("peer_same_group", str_eq(g1, g2), 1);

    /* a slave of it */
    expect("slave_make", pm_mount(0, B "/outside", 0, MS_SLAVE, 0), 0);
    struct pm_mi sl = get(B "/outside");
    expect("slave_master_only", pm_group(sl.optional, "master:") != 0, 1);
    expect("slave_not_shared", pm_find(sl.optional, "shared") < 0, 1);
    expect("slave_master_is_group", str_eq(group_of(&sl, "master:", g3), g1), 1);
    expect("slave_ten_plus_one", sl.nfields, 11);

    /* a slave that is shared again */
    expect("slave_shared_again", pm_mount(0, B "/outside", 0, MS_SHARED, 0), 0);
    struct pm_mi both = get(B "/outside");
    expect("both_fields", both.nfields, 12);
    expect("both_shared_first", pm_starts(both.optional, "shared:"), 1);
    expect("both_has_master", pm_find(both.optional, " master:") > 0, 1);
    group_of(&both, "shared:", g2);
    expect("both_new_group", str_eq(g2, g1), 0);
    expect("both_master_old_group", str_eq(group_of(&both, "master:", g3), g1), 1);

    /* back to private */
    expect("slave_to_private", pm_mount(0, B "/outside", 0, MS_PRIVATE, 0), 0);
    struct pm_mi pv = get(B "/outside");
    expect("private_again_no_optional", pv.optional[0], 0);
    expect("private_again_ten_fields", pv.nfields, 10);
    m = get(T);
    expect("original_still_shared", pm_group(m.optional, "shared:") != 0, 1);

    /* unbindable */
    expect("bind_for_unbindable", pm_mount(T "/priv", T "/unb", 0, MS_BIND, 0), 0);
    expect("make_unbindable", pm_mount(0, T "/unb", 0, MS_UNBINDABLE, 0), 0);
    struct pm_mi ub = get(T "/unb");
    expect("unbindable_optional", str_eq(ub.optional, "unbindable"), 1);
    expect("unbindable_fields", ub.nfields, 11);
    expect("unbindable_to_shared", pm_mount(0, T "/unb", 0, MS_SHARED, 0), 0);
    ub = get(T "/unb");
    expect("shared_replaces_unbindable", pm_group(ub.optional, "shared:") != 0, 1);

    /* a mount made below a shared mount is shared as well; the type can
     * be set for the whole tree */
    expect("mount_below_shared", pm_mount("pkbelow", T "/peer", "tmpfs", 0, 0), 0);
    struct pm_mi below = get(T "/peer");
    expect("below_inherits_shared", pm_group(below.optional, "shared:") != 0, 1);
    expect("private_rec", pm_mount(0, T, 0, MS_PRIVATE | MS_REC, 0), 0);
    m = get(T);
    below = get(T "/peer");
    expect("rec_top_private", m.optional[0], 0);
    expect("rec_below_private", below.optional[0], 0);
    expect("shared_rec", pm_mount(0, T, 0, MS_SHARED | MS_REC, 0), 0);
    m = get(T);
    below = get(T "/peer");
    expect("rec_top_shared", pm_group(m.optional, "shared:") != 0, 1);
    expect("rec_below_shared", pm_group(below.optional, "shared:") != 0, 1);
    expect("rec_groups_differ", str_eq(group_of(&m, "shared:", g1), group_of(&below, "shared:", g2)), 0);
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    a_rmdir(B "/outside");
    a_rmdir(T);
    a_rmdir(B);
    pm_in_ns(body);
    a_rmdir(B "/outside");
    a_rmdir(T);
    expect("cleanup", a_rmdir(B), 0);
    return finish();
}
