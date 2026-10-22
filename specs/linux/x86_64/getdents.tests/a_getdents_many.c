/* getdents on a directory of 300 files read with buffers of 100, 1000,
 * 4096 and 32768 bytes: every call returns whole records, never more
 * bytes than the buffer, at least one record while any is left; over the
 * whole listing every name comes exactly once; then 0. */
#include "a_sys.h"

#define SYS_getdents 78

struct g_dirent {
    u64 d_ino;
    u64 d_off;
    u16 d_reclen;
    char d_name[];
};

#define D "/tmp/a_getdents_many"
#define NF 300

static char buf[32768] __attribute__((aligned(8)));
static int seen[NF];

static void name_of(char *out, int i) {
    out[0] = 'f';
    out[1] = (char)('0' + i / 100);
    out[2] = (char)('0' + i / 10 % 10);
    out[3] = (char)('0' + i % 10);
    for (int k = 0; k < i % 7; k++) out[4 + k] = 'x';
    out[4 + i % 7] = 0;
}

static int index_of(const char *s) {
    if (s[0] != 'f' || s[1] < '0' || s[1] > '9' || s[2] < '0' || s[2] > '9' || s[3] < '0' || s[3] > '9') return -1;
    int i = (s[1] - '0') * 100 + (s[2] - '0') * 10 + (s[3] - '0');
    char want[16];
    if (i >= NF) return -1;
    name_of(want, i);
    return str_eq(s, want) ? i : -1;
}

static void pass(i64 fd, i64 size, const char *tag) {
    char c[64];
    int dots = 0, bad = 0, whole = 1, calls = 0;
    i64 n;
    for (int i = 0; i < NF; i++) seen[i] = 0;
    expect(str_cat2(c, tag, "_rewind"), syscall3(SYS_lseek, fd, 0, SEEK_SET), 0);
    while ((n = syscall3(SYS_getdents, fd, buf, size)) > 0 && calls < 10000) {
        calls++;
        whole &= n <= size;
        i64 o = 0;
        while (o < n) {
            struct g_dirent *d = (struct g_dirent *)(buf + o);
            if (d->d_reclen == 0 || o + d->d_reclen > n) {
                whole = 0;
                break;
            }
            whole &= d->d_reclen == ((18 + str_len(d->d_name) + 2 + 7) & ~7L);
            int i = index_of(d->d_name);
            if (i >= 0)
                seen[i]++;
            else if (str_eq(d->d_name, ".") || str_eq(d->d_name, ".."))
                dots++;
            else
                bad++;
            o += d->d_reclen;
        }
        whole &= o == n;
    }
    expect(str_cat2(c, tag, "_ends_with_0"), n, 0);
    check(str_cat2(c, tag, "_whole_records"), whole, 0);
    expect(str_cat2(c, tag, "_dot_and_dotdot"), dots, 2);
    expect(str_cat2(c, tag, "_no_other_names"), bad, 0);
    int once = 1;
    for (int i = 0; i < NF; i++) once &= seen[i] == 1;
    check(str_cat2(c, tag, "_each_file_once"), once, 0);
}

int test_main(void) {
    char p[96], nm[16];
    int made = 1;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    for (int i = 0; i < NF; i++) {
        name_of(nm, i);
        made &= a_make_file(str_cat2(p, D "/", nm), "", 0644) == 0;
    }
    check("setup_files", made, 0);

    i64 fd = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    pass(fd, 100, "size_100");
    pass(fd, 1000, "size_1000");
    pass(fd, 4096, "size_4096");
    pass(fd, 32768, "size_32768");
    a_close(fd);

    int gone = 1;
    for (int i = 0; i < NF; i++) {
        name_of(nm, i);
        gone &= a_unlink(str_cat2(p, D "/", nm)) == 0;
    }
    check("cleanup_files", gone, 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
