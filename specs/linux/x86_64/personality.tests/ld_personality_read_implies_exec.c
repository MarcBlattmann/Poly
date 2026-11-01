/* With READ_IMPLIES_EXEC (0x0400000) in the persona, a mapping requested
 * with PROT_READ is made readable and executable: /proc/self/maps shows
 * it as "r-xp". mprotect(PROT_READ) likewise gives r-x, and
 * PROT_READ|PROT_WRITE gives rwx. Without the flag the same requests give
 * "r--p" and "rw-p". PROT_NONE stays "---p" either way. Setting the flag
 * does not change mappings that already exist.
 * poly-diff: syscalls=names (the first call returns the inherited persona)
 */
#include "a_sys.h"

#define SYS_personality 135
#define QUERY 0xffffffffUL
#define READ_IMPLIES_EXEC 0x0400000UL

static i64 pers(u64 v) { return syscall1(SYS_personality, v); }

static char maps[65536];

/* The four permission characters of the mapping that starts at addr. */
static void perms_of(u64 addr, char out[5]) {
    out[0] = '?';
    out[1] = out[2] = out[3] = '?';
    out[4] = 0;
    i64 fd = a_open("/proc/self/maps", O_RDONLY, 0);
    if (fd < 0) return;
    i64 n = 0, k;
    while ((k = a_read(fd, maps + n, sizeof maps - 1 - n)) > 0) n += k;
    maps[n] = 0;
    a_close(fd);
    char *p = maps;
    while (*p) {
        u64 start = 0;
        char *q = p;
        for (;; q++) {
            char ch = *q;
            if (ch >= '0' && ch <= '9') start = start * 16 + (u64)(ch - '0');
            else if (ch >= 'a' && ch <= 'f') start = start * 16 + (u64)(ch - 'a' + 10);
            else break;
        }
        if (start == addr) {
            while (*q && *q != ' ') q++;
            if (*q == ' ') q++;
            for (int i = 0; i < 4 && q[i]; i++) out[i] = q[i];
            return;
        }
        while (*p && *p != '\n') p++;
        if (*p) p++;
    }
}

static u64 map(u64 prot) { return (u64)syscall6(SYS_mmap, 0, A_PAGE, prot, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0); }

int test_main(void) {
    i64 orig = pers(QUERY);
    char p[5];

    pers(0);
    u64 a = map(PROT_READ);
    perms_of(a, p);
    check("plain_read_is_r", str_eq(p, "r--p"), p[2]);
    u64 b = map(PROT_READ | PROT_WRITE);
    perms_of(b, p);
    check("plain_rw_is_rw", str_eq(p, "rw-p"), p[2]);

    pers(READ_IMPLIES_EXEC);
    perms_of(b, p);
    check("existing_mapping_unchanged", str_eq(p, "rw-p"), p[2]);
    u64 c = map(PROT_READ);
    perms_of(c, p);
    check("rie_read_is_rx", str_eq(p, "r-xp"), p[2]);
    u64 d = map(PROT_READ | PROT_WRITE);
    perms_of(d, p);
    check("rie_rw_is_rwx", str_eq(p, "rwxp"), p[2]);
    u64 e = map(PROT_NONE);
    perms_of(e, p);
    check("rie_none_stays_none", str_eq(p, "---p"), p[0]);
    check("mprotect_ok", syscall3(SYS_mprotect, a, A_PAGE, PROT_READ) == 0, 0);
    perms_of(a, p);
    check("rie_mprotect_read_is_rx", str_eq(p, "r-xp"), p[2]);

    pers(0);
    syscall3(SYS_mprotect, c, A_PAGE, PROT_READ);
    perms_of(c, p);
    check("plain_mprotect_read_is_r", str_eq(p, "r--p"), p[2]);
    u64 f = map(PROT_READ);
    perms_of(f, p);
    check("plain_again_r", str_eq(p, "r--p"), p[2]);

    pers((u64)orig);
    check("restored", pers(QUERY) == orig, 0);
    return finish();
}
