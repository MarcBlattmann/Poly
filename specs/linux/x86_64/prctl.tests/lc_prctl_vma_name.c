/* prctl(PR_SET_VMA 0x53564d41, PR_SET_VMA_ANON_NAME 0, addr, len, name)
 * names a range of anonymous private mappings (Chromium names its
 * allocator's regions this way): 0, and /proc/self/maps shows the range's
 * line ending in "[anon:<name>]"; a NULL name removes the name again
 * (0). The name may hold printable characters but not \ ` $ [ ]
 * (EINVAL), and at most 79 bytes (80 or more: EINVAL). addr must be page
 * aligned (EINVAL); a range with an unmapped page is ENOMEM; a file mapping
 * is EBADF; a sub-option other than PR_SET_VMA_ANON_NAME is EINVAL.
 * poly-diff: syscalls=names (the maps buffer's reads depend on the layout)
 */
#include "a_sys.h"

#define SYS_prctl 157
#define PR_SET_VMA 0x53564d41
#define PR_SET_VMA_ANON_NAME 0
#define PAGE 4096

static i64 name_vma(u64 addr, u64 len, const char *name) {
    return syscall5(SYS_prctl, PR_SET_VMA, PR_SET_VMA_ANON_NAME, addr, len, (u64)name);
}

static char maps[65536];

/* Whether /proc/self/maps has a line containing `needle`. */
static int maps_has(const char *needle) {
    i64 fd = a_open("/proc/self/maps", O_RDONLY, 0);
    if (fd < 0) return -1;
    i64 n = 0, r;
    while ((r = a_read(fd, maps + n, sizeof maps - 1 - n)) > 0) n += r;
    a_close(fd);
    maps[n] = 0;
    size_t k = str_len(needle);
    for (i64 i = 0; i + (i64)k <= n; i++)
        if (mem_eq(maps + i, needle, k)) return 1;
    return 0;
}

int test_main(void) {
    i64 m = syscall6(SYS_mmap, 0, 3 * PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    check("mmap", m > 0, m);
    u64 a = (u64)m;
    expect("name", name_vma(a, 3 * PAGE, "poly test-region_1"), 0);
    check("maps_shows_name", maps_has("[anon:poly test-region_1]") == 1, 0);
    expect("rename_middle_page", name_vma(a + PAGE, PAGE, "poly-mid"), 0);
    check("maps_shows_both", maps_has("[anon:poly-mid]") == 1 && maps_has("[anon:poly test-region_1]") == 1, 0);
    expect("clear", name_vma(a, 3 * PAGE, 0), 0);
    check("maps_name_gone", maps_has("[anon:poly-mid]") == 0 && maps_has("[anon:poly test-region_1]") == 0, 0);

    char name79[80], name80[81];
    for (int i = 0; i < 79; i++) name79[i] = 'n';
    name79[79] = 0;
    for (int i = 0; i < 80; i++) name80[i] = 'n';
    name80[80] = 0;
    expect("name_79_bytes", name_vma(a, PAGE, name79), 0);
    expect("name_80_bytes", name_vma(a, PAGE, name80), -EINVAL);
    expect("name_bracket", name_vma(a, PAGE, "a[b"), -EINVAL);
    expect("name_dollar", name_vma(a, PAGE, "a$b"), -EINVAL);
    expect("name_backslash", name_vma(a, PAGE, "a\\b"), -EINVAL);
    expect("name_newline", name_vma(a, PAGE, "a\nb"), -EINVAL);
    expect("unaligned", name_vma(a + 1, PAGE, "x"), -EINVAL);
    expect("bad_suboption", syscall5(SYS_prctl, PR_SET_VMA, 1, a, PAGE, (u64) "x"), -EINVAL);
    expect("name_fault", syscall5(SYS_prctl, PR_SET_VMA, PR_SET_VMA_ANON_NAME, a, PAGE, 8), -EFAULT);

    expect("munmap_last", syscall2(SYS_munmap, a + 2 * PAGE, PAGE), 0);
    expect("hole", name_vma(a, 3 * PAGE, "x"), -ENOMEM);

    i64 fd = a_open(a_self_path(), O_RDONLY, 0);
    i64 f = syscall6(SYS_mmap, 0, PAGE, PROT_READ, MAP_PRIVATE, fd, 0);
    a_close(fd);
    check("file_mmap", f > 0, f);
    expect("file_mapping", name_vma((u64)f, PAGE, "x"), -EBADF);
    return finish();
}
