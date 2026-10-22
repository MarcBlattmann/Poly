/* PR_SET_VMA with PR_SET_VMA_ANON_NAME names a range of private
 * anonymous memory: /proc/self/maps then shows "[anon:<name>]" for it,
 * splitting mappings at the range's ends. arg3 is the page-aligned
 * start, arg4 the length (rounded up to whole pages; 0 does nothing and
 * succeeds), arg5 the name: at most 79 bytes of printable characters
 * other than [ ] \ $ and ` (else EINVAL). An empty name is a name too
 * ("[anon:]"); NULL removes the name. An unaligned start or an unknown
 * sub-operation is EINVAL; a range with unmapped parts is ENOMEM; an
 * unreadable name is EFAULT.
 * poly-diff: syscalls=off (mapping addresses, and so the size of /proc/self/maps, differ between runs)
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_prctl 157
/* linux/prctl.h */
#define PR_SET_VMA 0x53564d41
#define PR_SET_VMA_ANON_NAME 0

static i64 pr(i64 o, i64 a, i64 b, i64 c, i64 d) { return syscall5(SYS_prctl, o, a, b, c, d); }

static char maps[65536];

/* Hex text of v (lower case, no leading zeros), as in /proc/self/maps. */
static void hex(char *out, u64 v) {
    char tmp[17];
    int n = 0;
    do tmp[n++] = "0123456789abcdef"[v & 15];
    while (v >>= 4);
    while (n) *out++ = tmp[--n];
    *out = 0;
}

static int starts_with(const char *s, const char *p) {
    while (*p)
        if (*s++ != *p++) return 0;
    return 1;
}

/* Whether /proc/self/maps shows the mapping from `a` to `b` with name
 * "[anon:<want>]" (no name when want is 0): 1 if so, 0 if not, -1 if
 * there is no such mapping. */
static int maps_name(u64 a, u64 b, const char *want) {
    i64 fd = a_open("/proc/self/maps", O_RDONLY, 0);
    if (fd < 0) return -1;
    i64 n = 0, r;
    while ((r = a_read(fd, maps + n, sizeof maps - 1 - n)) > 0) n += r;
    maps[n] = 0;
    a_close(fd);
    char range[40], *p = range;
    hex(p, a);
    while (*p) p++;
    *p++ = '-';
    hex(p, b);
    for (char *line = maps; *line;) {
        char *end = line;
        while (*end && *end != '\n') end++;
        if (starts_with(line, range) && line[str_len(range)] == ' ') {
            /* An unnamed anonymous mapping's line ends in " 0 ". */
            if (!want) return end - line >= 3 && mem_eq(end - 3, " 0 ", 3);
            char exp[128];
            str_cat2(exp, " [anon:", want);
            size_t n = str_len(exp);
            exp[n++] = ']';
            return (size_t)(end - line) >= n && mem_eq(end - n, exp, n);
        }
        line = *end ? end + 1 : end;
    }
    return -1;
}

int test_main(void) {
    /* Four pages between two PROT_NONE guard pages, so they never merge
     * with a neighbouring mapping. */
    u64 g = (u64)syscall6(SYS_mmap, 0, 6 * A_PAGE, PROT_NONE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    u64 m = g + A_PAGE;
    expect("mprotect", syscall3(SYS_mprotect, m, 4 * A_PAGE, PROT_READ | PROT_WRITE), 0);
    expect("name whole range", pr(PR_SET_VMA, PR_SET_VMA_ANON_NAME, m, 4 * A_PAGE, (i64) "first name"), 0);
    expect("maps: whole range", maps_name(m, m + 4 * A_PAGE, "first name"), 1);
    expect("name page 1", pr(PR_SET_VMA, PR_SET_VMA_ANON_NAME, m + A_PAGE, A_PAGE, (i64) "second"), 0);
    expect("maps: page 0 keeps name", maps_name(m, m + A_PAGE, "first name"), 1);
    expect("maps: page 1 split off", maps_name(m + A_PAGE, m + 2 * A_PAGE, "second"), 1);
    expect("maps: pages 2-3", maps_name(m + 2 * A_PAGE, m + 4 * A_PAGE, "first name"), 1);

    expect("length rounded up", pr(PR_SET_VMA, PR_SET_VMA_ANON_NAME, m + 3 * A_PAGE, 100, (i64) "rounded"), 0);
    expect("maps: rounded page", maps_name(m + 3 * A_PAGE, m + 4 * A_PAGE, "rounded"), 1);
    expect("empty name", pr(PR_SET_VMA, PR_SET_VMA_ANON_NAME, m + 2 * A_PAGE, A_PAGE, (i64) ""), 0);
    expect("maps: empty name shown", maps_name(m + 2 * A_PAGE, m + 3 * A_PAGE, ""), 1);
    expect("NULL name removes", pr(PR_SET_VMA, PR_SET_VMA_ANON_NAME, m + A_PAGE, A_PAGE, 0), 0);
    expect("maps: page 1 unnamed", maps_name(m + A_PAGE, m + 2 * A_PAGE, 0), 1);

    char longname[100];
    for (int i = 0; i < 99; i++) longname[i] = 'x';
    longname[79] = 0;
    expect("79-byte name", pr(PR_SET_VMA, PR_SET_VMA_ANON_NAME, m, A_PAGE, (i64)longname), 0);
    expect("maps: 79-byte name", maps_name(m, m + A_PAGE, longname), 1);
    longname[79] = 'x';
    longname[80] = 0;
    expect("80-byte name", pr(PR_SET_VMA, PR_SET_VMA_ANON_NAME, m, A_PAGE, (i64)longname), -EINVAL);
    expect("bad char [", pr(PR_SET_VMA, PR_SET_VMA_ANON_NAME, m, A_PAGE, (i64) "a[b"), -EINVAL);
    expect("bad char ]", pr(PR_SET_VMA, PR_SET_VMA_ANON_NAME, m, A_PAGE, (i64) "a]b"), -EINVAL);
    expect("bad char backslash", pr(PR_SET_VMA, PR_SET_VMA_ANON_NAME, m, A_PAGE, (i64) "a\\b"), -EINVAL);
    expect("bad char $", pr(PR_SET_VMA, PR_SET_VMA_ANON_NAME, m, A_PAGE, (i64) "a$b"), -EINVAL);
    expect("bad char `", pr(PR_SET_VMA, PR_SET_VMA_ANON_NAME, m, A_PAGE, (i64) "a`b"), -EINVAL);
    expect("bad char newline", pr(PR_SET_VMA, PR_SET_VMA_ANON_NAME, m, A_PAGE, (i64) "a\nb"), -EINVAL);
    expect("maps: name kept", maps_name(m, m + A_PAGE, "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"),
           1);
    expect("unaligned start", pr(PR_SET_VMA, PR_SET_VMA_ANON_NAME, m + 1, A_PAGE, (i64) "q"), -EINVAL);
    expect("length 0", pr(PR_SET_VMA, PR_SET_VMA_ANON_NAME, m, 0, (i64) "q"), 0);
    expect("unknown sub-operation", pr(PR_SET_VMA, 1, m, A_PAGE, (i64) "q"), -EINVAL);
    expect("name unreadable", pr(PR_SET_VMA, PR_SET_VMA_ANON_NAME, m, A_PAGE, 8), -EFAULT);
    syscall2(SYS_munmap, m + 3 * A_PAGE, A_PAGE);
    expect("range runs into a hole", pr(PR_SET_VMA, PR_SET_VMA_ANON_NAME, m + 2 * A_PAGE, 2 * A_PAGE, (i64) "q"),
           -ENOMEM);
    return finish();
}
