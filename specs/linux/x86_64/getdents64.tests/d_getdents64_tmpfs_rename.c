/* getdents64 on tmpfs after rename: the renamed entry moves to the front
 * of the listing, whether it was renamed within the directory or came
 * from another one. Renamed to a new name it gets a new cookie (the
 * newest); renamed over an existing entry it takes that entry's cookie.
 * RENAME_EXCHANGE moves both names to the front (the second path's name
 * first), each keeping its cookie. A rename to the same name, or between
 * two links of one file, changes nothing. */
#include "a_sys.h"

/* linux/magic.h */
#define TMPFS_MAGIC 0x01021994

static char buf[8192];

/* The records of the last read: name, d_off and d_type of each. */
#define MAXR 64
static char rec_name[MAXR][48];
static i64 rec_off[MAXR];
static u8 rec_type[MAXR];
static int nrec;

static A_UNUSED void take(i64 n) {
    for (i64 o = 0; o < n;) {
        struct a_dirent64 *d = (struct a_dirent64 *)(buf + o);
        if (nrec < MAXR) {
            str_cpy(rec_name[nrec], d->d_name);
            rec_off[nrec] = d->d_off;
            rec_type[nrec] = d->d_type;
            nrec++;
        }
        o += d->d_reclen;
    }
}

/* getdents64 calls of `size` bytes from the current position until one
 * returns 0 (or an error, which is returned). */
static A_UNUSED i64 read_rest(i64 fd, i64 size) {
    i64 n;
    nrec = 0;
    while ((n = syscall3(SYS_getdents64, fd, buf, size)) > 0) take(n);
    return n;
}

/* One getdents64 call of `size` bytes; returns its result. */
static A_UNUSED i64 read_call(i64 fd, i64 size) {
    nrec = 0;
    i64 n = syscall3(SYS_getdents64, fd, buf, size);
    if (n > 0) take(n);
    return n;
}

/* Rewind (lseek to 0) and read the whole listing. */
static A_UNUSED i64 read_all(i64 fd) {
    syscall3(SYS_lseek, fd, 0, SEEK_SET);
    return read_rest(fd, sizeof buf);
}

static char text[2048];

/* The names of the records, in the order returned, space-separated. */
static A_UNUSED const char *names_text(void) {
    char *p = text;
    *p = 0;
    for (int i = 0; i < nrec; i++) {
        if (i) p = str_cpy(p, " ");
        p = str_cpy(p, rec_name[i]);
    }
    return text;
}

/* Each record's d_off as its rank among the distinct d_off values of
 * these records (1 = the smallest): the raw values are not compared,
 * only their order. */
static A_UNUSED const char *ranks_text(void) {
    char *p = text, num[24];
    *p = 0;
    for (int i = 0; i < nrec; i++) {
        int rank = 1;
        for (int j = 0; j < nrec; j++) {
            if (rec_off[j] >= rec_off[i]) continue;
            int first = 1;
            for (int k = 0; k < j; k++)
                if (rec_off[k] == rec_off[j]) first = 0;
            rank += first;
        }
        if (i) p = str_cpy(p, " ");
        p = str_cpy(p, a_utoa(num, (u64)rank));
    }
    return text;
}

static char label_buf[128];

/* Print "<label>: <names>" and check the names; then, unless want_ranks
 * is 0, print "<label> d_off: <ranks>" and check those. */
static A_UNUSED void show(const char *label, const char *want_names, const char *want_ranks) {
    const char *got = names_text();
    put(label);
    put(": ");
    put(got);
    put("\n");
    check(str_cat2(label_buf, label, "_names"), str_eq(got, want_names), nrec);
    if (!want_ranks) return;
    got = ranks_text();
    put(label);
    put(" d_off: ");
    put(got);
    put("\n");
    check(str_cat2(label_buf, label, "_d_off"), str_eq(got, want_ranks), nrec);
}

/* "<dir>/<name>" in one of four rotating buffers. */
static A_UNUSED const char *at(const char *dir, const char *name) {
    static char paths[4][256];
    static int next;
    char *p = paths[next++ & 3];
    str_cpy(str_cpy(str_cpy(p, dir), "/"), name);
    return p;
}

static A_UNUSED i64 mk(const char *dir, const char *name) { return a_make_file(at(dir, name), "", 0644); }

static A_UNUSED i64 mv(const char *from_dir, const char *from, const char *to_dir, const char *to) {
    return syscall2(SYS_rename, at(from_dir, from), at(to_dir, to));
}

/* Create each space-separated name of `names` in `dir`, in that order;
 * returns how many failed. */
static A_UNUSED int mk_all(const char *dir, const char *names) {
    char one[32];
    int bad = 0;
    while (*names) {
        int n = 0;
        while (*names && *names != ' ') one[n++] = *names++;
        one[n] = 0;
        while (*names == ' ') names++;
        bad += mk(dir, one) != 0;
    }
    return bad;
}

/* Unlink each space-separated name of `names` in `dir`; returns how many
 * failed. */
static A_UNUSED int rm_all(const char *dir, const char *names) {
    char one[32];
    int bad = 0;
    while (*names) {
        int n = 0;
        while (*names && *names != ' ') one[n++] = *names++;
        one[n] = 0;
        while (*names == ' ') names++;
        bad += a_unlink(at(dir, one)) != 0;
    }
    return bad;
}

/* The test directories are on the tmpfs at /dev/shm (on Linux and on
 * Poly); say so if it is not one. */
static A_UNUSED void check_tmpfs(void) {
    struct a_statfs sf;
    i64 r = syscall2(SYS_statfs, "/dev/shm", &sf);
    check("setup_dev_shm_is_tmpfs", r == 0 && sf.f_type == TMPFS_MAGIC, r == 0 ? sf.f_type : r);
}

#define D "/dev/shm/d_getdents64_tmpfs_rename"
#define O "/dev/shm/d_getdents64_tmpfs_rename_other"

int test_main(void) {
    check_tmpfs();
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_mkdir_other", a_mkdir(O, 0755), 0);
    expect("setup_files", mk_all(D, "a b c d e"), 0);
    i64 fd = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    expect("read_created", read_all(fd), 0);
    show("created", ". .. e d c b a", "1 6 5 4 3 2 7");

    expect("rename_new_name", mv(D, "b", D, "b2"), 0);
    expect("read_renamed", read_all(fd), 0);
    show("renamed", ". .. b2 e d c a", "1 6 5 4 3 2 7");

    /* d over a: "a" (now d's file) goes to the front with a's old cookie,
     * the smallest. */
    expect("rename_over", mv(D, "d", D, "a"), 0);
    expect("read_over", read_all(fd), 0);
    show("over", ". .. a b2 e c", "1 2 5 4 3 6");

    expect("setup_other_z", mk(O, "z"), 0);
    expect("rename_in", mv(O, "z", D, "z"), 0);
    expect("read_in", read_all(fd), 0);
    show("moved_in", ". .. z a b2 e c", "1 6 2 5 4 3 7");

    expect("rename_out", mv(D, "e", O, "e"), 0);
    expect("read_out", read_all(fd), 0);
    show("moved_out", ". .. z a b2 c", "1 5 2 4 3 6");

    /* From another directory over an existing entry: c's cookie. */
    expect("setup_other_w", mk(O, "w"), 0);
    expect("rename_in_over", mv(O, "w", D, "c"), 0);
    expect("read_in_over", read_all(fd), 0);
    show("moved_in_over", ". .. c z a b2", "1 3 5 2 4 6");

    /* Away and back: a new cookie each time. */
    expect("rename_away", mv(D, "a", D, "a_tmp"), 0);
    expect("rename_back", mv(D, "a_tmp", D, "a"), 0);
    expect("read_back", read_all(fd), 0);
    show("round_trip", ". .. a c z b2", "1 5 2 4 3 6");

    /* Exchange z and b2: "b2" first, then "z", then the rest. */
    expect("exchange", syscall5(SYS_renameat2, AT_FDCWD, D "/z", AT_FDCWD, D "/b2", RENAME_EXCHANGE), 0);
    expect("read_exchange", read_all(fd), 0);
    show("exchanged", ". .. b2 z a c", "1 3 4 5 2 6");

    /* Nothing to do: the same name; two links of the same file. */
    expect("rename_same", mv(D, "c", D, "c"), 0);
    expect("setup_link", syscall5(SYS_linkat, AT_FDCWD, D "/c", AT_FDCWD, D "/c_link", 0), 0);
    expect("rename_links", mv(D, "c", D, "c_link"), 0);
    expect("read_noop", read_all(fd), 0);
    show("no_op", ". .. c_link b2 z a c", 0);

    /* A directory renamed within: the same rules. */
    expect("setup_subdir", a_mkdir(D "/sub", 0755), 0);
    expect("rename_dir", mv(D, "sub", D, "sub2"), 0);
    expect("rename_file_after", mv(D, "a", D, "a3"), 0);
    expect("read_dir", read_all(fd), 0);
    show("dir_renamed", ". .. a3 sub2 c_link b2 z c", 0);
    a_close(fd);

    expect("cleanup_sub", a_rmdir(D "/sub2"), 0);
    expect("cleanup_files", rm_all(D, "a3 c_link b2 z c"), 0);
    expect("cleanup_other", a_unlink(O "/e"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    expect("cleanup_rmdir_other", a_rmdir(O), 0);
    return finish();
}
