/* getdents64 on tmpfs with 40 entries: newest first; after every other
 * one is removed and five are created, the five come first (newest
 * first) and the survivors keep their order; read with a 100-byte buffer
 * (four 24-byte records a call) the listing is the same. */
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

#define D "/dev/shm/d_getdents64_tmpfs_forty"

static char name_of[8];

/* "f" and two digits. */
static const char *fname(int i) {
    name_of[0] = 'f';
    name_of[1] = (char)('0' + i / 10);
    name_of[2] = (char)('0' + i % 10);
    name_of[3] = 0;
    return name_of;
}

static char want_text[1024];

/* ". ..", then `extra`, then f<from>, f<from - step>, ... down to 0. */
static const char *want_list(const char *extra, int from, int step) {
    char *p = str_cpy(want_text, ". ..");
    if (*extra) p = str_cpy(str_cpy(p, " "), extra);
    for (int i = from; i >= 0; i -= step) p = str_cpy(str_cpy(p, " "), fname(i));
    return want_text;
}

int test_main(void) {
    check_tmpfs();
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    int bad = 0;
    for (int i = 0; i < 40; i++) bad += mk(D, fname(i)) != 0;
    expect("setup_files", bad, 0);
    i64 fd = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    expect("read_all", read_all(fd), 0);
    show("all40", want_list("", 39, 1), 0);
    /* d_off: "." smallest, then from the newest entry's down to the
     * oldest's, the end largest. */
    int desc = rec_off[0] < rec_off[nrec - 1];
    for (int i = 1; i + 2 < nrec; i++) desc &= rec_off[i] > rec_off[i + 1] && rec_off[i] > rec_off[0];
    for (int i = 0; i + 1 < nrec; i++) desc &= rec_off[nrec - 1] > rec_off[i];
    check("all40_d_off_descending", desc, 0);

    bad = 0;
    for (int i = 0; i < 40; i += 2) bad += a_unlink(at(D, fname(i))) != 0;
    expect("unlink_even", bad, 0);
    expect("create_g", mk_all(D, "g0 g1 g2 g3 g4"), 0);
    expect("read_after", read_all(fd), 0);
    show("after", want_list("g4 g3 g2 g1 g0", 39, 2), 0);
    desc = 1;
    for (int i = 1; i + 2 < nrec; i++) desc &= rec_off[i] > rec_off[i + 1] && rec_off[i] > rec_off[0];
    for (int i = 0; i + 1 < nrec; i++) desc &= rec_off[nrec - 1] > rec_off[i];
    check("after_d_off_descending", desc, 0);

    /* With four records a call. */
    syscall3(SYS_lseek, fd, 0, SEEK_SET);
    char seen[1024];
    char *p = seen;
    *p = 0;
    int calls = 0, fours = 1;
    i64 n;
    while ((n = read_call(fd, 100)) > 0) {
        calls++;
        fours &= n == 96 || n == 24 * nrec;
        for (int i = 0; i < nrec; i++) p = str_cpy(str_cpy(p, p == seen ? "" : " "), rec_name[i]);
    }
    expect("small_end_0", n, 0);
    expect("small_calls", calls, 7);
    check("small_four_per_call", fours, 0);
    check("small_same_names", str_eq(seen, want_list("g4 g3 g2 g1 g0", 39, 2)), 0);
    a_close(fd);

    bad = 0;
    for (int i = 1; i < 40; i += 2) bad += a_unlink(at(D, fname(i))) != 0;
    expect("cleanup_odd", bad, 0);
    expect("cleanup_g", rm_all(D, "g0 g1 g2 g3 g4"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
