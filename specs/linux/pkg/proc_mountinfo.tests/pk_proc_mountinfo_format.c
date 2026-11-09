/* /proc/self/mountinfo (also /proc/<pid>/mountinfo) is a read-only regular
 * file (mode 0444) with one line per mount of the reader's namespace:
 *   id parent major:minor root mountpoint options [optional...] - fstype source superoptions
 * separated by single spaces, ending with a newline. A tmpfs mounted with
 * data "size=1m,mode=700,nr_inodes=100" shows root "/", the mount point as
 * given, options "rw,relatime", no optional field (a private mount), device
 * "0:<n>", fstype tmpfs, the source string as passed, and superoptions that
 * start with "rw" and contain size=1024k, mode=700, nr_inodes=100. A new
 * mount is listed at once, an unmounted one is gone. Reading in tiny pieces
 * gives the same text as one big read; /proc/self/mountinfo and
 * /proc/<own pid>/mountinfo are identical. /proc/self/mounts lists the same
 * mount as "source mountpoint fstype options 0 0" where options are the
 * mount options followed by the superblock's (without repeating rw/ro), and
 * /proc/mounts is a link to self/mounts.
 * poly-diff: needs=root (mount needs CAP_SYS_ADMIN)
 * poly-diff: syscalls=off (the mount runs in a child in a mount namespace of its own)
 */
#include "p_mnt.h"

#define B "/tmp/pk_mif"
#define T B "/t"

static char first[PM_MI_MAX];

static int body(void) {
    struct a_stat st;
    expect("stat_mountinfo", a_stat_path("/proc/self/mountinfo", &st), 0);
    expect("mountinfo_regular", st.st_mode & S_IFMT, S_IFREG);
    expect("mountinfo_mode", st.st_mode & 07777, 0444);
    expect("mountinfo_size_zero", st.st_size, 0);
    expect("stat_mounts", a_stat_path("/proc/self/mounts", &st), 0);
    expect("mounts_mode", st.st_mode & 07777, 0444);
    char link[32];
    memset(link, 0, sizeof link);
    expect("proc_mounts_link_len", syscall3(SYS_readlink, "/proc/mounts", link, sizeof link), 11);
    check("proc_mounts_link", str_eq(link, "self/mounts"), 0);

    struct pm_mi mi;
    expect("load_before", pm_mi_load("/proc/self/mountinfo") > 0, 1);
    int lines, bad;
    pm_mi_count(&lines, &bad);
    check("has_lines", lines > 0, lines);
    expect("all_lines_well_formed", bad, 0);
    expect("ends_with_newline", pm_mi_text[str_len(pm_mi_text) - 1], '\n');
    expect("not_mounted_yet", pm_mi_get(T, 0, &mi), 0);
    expect("root_is_listed", pm_mi_get("/", 0, &mi), 1);
    expect("proc_is_listed", pm_mi_get("/proc", 0, &mi), 1);
    expect("proc_fstype", str_eq(mi.fstype, "proc"), 1);

    pm_mkdirs(T);
    expect("mount", pm_mount("pkfmt", T, "tmpfs", 0, "size=1m,mode=700,nr_inodes=100"), 0);
    expect("load_after", pm_mi_load("/proc/self/mountinfo") > 0, 1);
    pm_mi_count(&lines, &bad);
    expect("still_well_formed", bad, 0);
    expect("listed_at_once", pm_mi_get(T, -1, &mi), 1);
    expect("fields_without_optional", mi.nfields, 10);
    expect("sep_index", mi.sep, 6);
    expect("root_field", str_eq(mi.root, "/"), 1);
    expect("options_field", str_eq(mi.opts, "rw,relatime"), 1);
    expect("no_optional_fields", mi.optional[0], 0);
    expect("fstype_field", str_eq(mi.fstype, "tmpfs"), 1);
    expect("source_field", str_eq(mi.source, "pkfmt"), 1);
    check("dev_anonymous_major_0", pm_starts(mi.dev, "0:") && mi.dev[2] != 0, 0);
    expect("super_starts_rw", pm_starts(mi.super, "rw,"), 1);
    expect("super_size", pm_has_item(mi.super, "size=1024k"), 1);
    expect("super_mode", pm_has_item(mi.super, "mode=700"), 1);
    expect("super_nr_inodes", pm_has_item(mi.super, "nr_inodes=100"), 1);
    int same_numbers = 1;
    for (const char *c = mi.id; *c; c++) same_numbers &= *c >= '0' && *c <= '9';
    expect("id_numeric", same_numbers, 1);
    struct pm_mi parent;
    /* the mount's parent is the mount the directory was on */
    int found_parent = 0;
    {
        const char *p = pm_mi_text;
        struct pm_mi cur;
        while (*p) {
            p = pm_mi_parse(p, &cur);
            if (str_eq(cur.id, mi.parent)) {
                found_parent = 1;
                parent = cur;
            }
        }
    }
    expect("parent_exists", found_parent, 1);
    expect("parent_contains_mountpoint", pm_starts(T, str_eq(parent.mp, "/") ? "/" : parent.mp), 1);

    /* stable under chunked reads, and the same through the pid */
    str_cpy(first, pm_mi_text);
    i64 fd = a_open("/proc/self/mountinfo", O_RDONLY, 0);
    size_t got = 0;
    char piece[7];
    for (;;) {
        i64 n = a_read(fd, piece, sizeof piece);
        if (n <= 0) break;
        if (got + (size_t)n < PM_MI_MAX - 1) memcpy(pm_mi_text + got, piece, (size_t)n);
        got += (size_t)n;
    }
    a_close(fd);
    pm_mi_text[got < PM_MI_MAX - 1 ? got : PM_MI_MAX - 1] = 0;
    expect("chunked_read_same", str_eq(first, pm_mi_text), 1);
    char path[48], num[24];
    str_cpy(str_cpy(str_cpy(path, "/proc/"), a_utoa(num, (u64)syscall0(SYS_getpid))), "/mountinfo");
    pm_mi_load(path);
    expect("pid_path_same", str_eq(first, pm_mi_text), 1);

    /* /proc/self/mounts */
    pm_mi_load("/proc/self/mounts");
    struct pm_mounts_line ml;
    expect("mounts_has_it", pm_mounts_get(T, -1, &ml), 1);
    expect("mounts_six_fields", ml.nfields, 6);
    expect("mounts_source", str_eq(ml.source, "pkfmt"), 1);
    expect("mounts_fstype", str_eq(ml.fstype, "tmpfs"), 1);
    expect("mounts_opts_start", pm_starts(ml.opts, "rw,relatime,"), 1);
    expect("mounts_opts_size", pm_has_item(ml.opts, "size=1024k"), 1);
    expect("mounts_opts_mode", pm_has_item(ml.opts, "mode=700"), 1);
    expect("mounts_zeros", str_eq(ml.zero1, "0") && str_eq(ml.zero2, "0"), 1);

    /* gone after umount */
    expect("umount", pm_umount(T, 0), 0);
    pm_mi_load("/proc/self/mountinfo");
    expect("unmounted_gone", pm_mi_get(T, 0, &mi), 0);
    pm_mi_load("/proc/self/mounts");
    expect("mounts_gone", pm_mounts_get(T, 0, &ml), 0);
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    a_rmdir(T);
    a_rmdir(B);
    pm_in_ns(body);
    a_rmdir(T);
    expect("cleanup", a_rmdir(B), 0);
    return finish();
}
