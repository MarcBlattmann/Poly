/* setxattr sizes and pointers: a value of more than 65536 bytes is E2BIG
 * (size_t compared in full, so 2^32 is too big as well); an unreadable value
 * (NULL with a size, an unmapped address, a buffer that runs into an
 * unmapped page) is EFAULT; an unreadable name or path is EFAULT; a name
 * whose first 256 bytes are readable but hold no NUL is ERANGE even when
 * the rest is unreadable. A 1000-byte value is stored exactly. */
#include "a_sys.h"

#define SYS_setxattr 188
#define SYS_getxattr 191
#define ENODATA 61
#define D "/tmp/a_setxattr_sizes"

static char big[70000];
static char got[1100];

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    for (int i = 0; i < 1000; i++) big[i] = (char)(i * 7 + 3);
    expect("set_1000", syscall5(SYS_setxattr, D "/f", "user.k", big, 1000, 0), 0);
    i64 n = syscall4(SYS_getxattr, D "/f", "user.k", got, sizeof got);
    check("get_1000", n == 1000 && mem_eq(got, big, 1000), n);
    expect("size_65537_e2big", syscall5(SYS_setxattr, D "/f", "user.b", big, 65537, 0), -E2BIG);
    expect("size_2p32_e2big", syscall5(SYS_setxattr, D "/f", "user.b", big, 0x100000000L, 0), -E2BIG);
    expect("size_max_e2big", syscall5(SYS_setxattr, D "/f", "user.b", big, -1L, 0), -E2BIG);
    expect("e2big_not_created", syscall4(SYS_getxattr, D "/f", "user.b", got, sizeof got), -ENODATA);
    expect("null_value_size_1_efault", syscall5(SYS_setxattr, D "/f", "user.b", 0, 1, 0), -EFAULT);
    expect("bad_value_efault", syscall5(SYS_setxattr, D "/f", "user.b", 8, 4, 0), -EFAULT);
    expect("bad_name_efault", syscall5(SYS_setxattr, D "/f", 8, "v", 1, 0), -EFAULT);
    expect("bad_path_efault", syscall5(SYS_setxattr, 8, "user.b", "v", 1, 0), -EFAULT);
    char *m = (char *)syscall6(SYS_mmap, 0, 8192, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    check("setup_mmap", (i64)m > 0, (i64)m);
    if ((i64)m > 0) {
        expect("setup_munmap", syscall2(SYS_munmap, m + 4096, 4096), 0);
        char *end = m + 4096;
        end[-4] = 'a', end[-3] = 'b', end[-2] = 'c', end[-1] = 'd';
        expect("value_into_hole_efault", syscall5(SYS_setxattr, D "/f", "user.v", end - 2, 4, 0), -EFAULT);
        expect("value_up_to_hole_ok", syscall5(SYS_setxattr, D "/f", "user.v", end - 4, 4, 0), 0);
        n = syscall4(SYS_getxattr, D "/f", "user.v", got, sizeof got);
        check("value_up_to_hole_kept", n == 4 && mem_eq(got, "abcd", 4), n);
        char *nm = end - 300;
        str_cpy(nm, "user.");
        for (int i = 5; i < 300; i++) nm[i] = 'x';
        expect("name_300_unterminated_erange", syscall5(SYS_setxattr, D "/f", nm, "v", 1, 0), -ERANGE);
        expect("name_256_unterminated_erange", syscall5(SYS_setxattr, D "/f", end - 256, "v", 1, 0), -ERANGE);
        for (int i = 0; i < 5; i++) end[-255 + i] = "user."[i];
        expect("name_255_unterminated_efault", syscall5(SYS_setxattr, D "/f", end - 255, "v", 1, 0), -EFAULT);
        expect("name_10_unterminated_efault", syscall5(SYS_setxattr, D "/f", end - 10, "v", 1, 0), -EFAULT);
        syscall2(SYS_munmap, m, 4096);
    }
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
