/* CAP_SYS_MODULE is the one capability that matters: a root child whose
 * effective set holds only CAP_SYS_MODULE gets past the privilege check
 * (ENOEXEC for an empty image), one whose effective set holds every other
 * capability but not CAP_SYS_MODULE gets EPERM.
 * poly-diff: needs=root (needs CAP_SYS_MODULE to start with)
 * poly-diff: syscalls=off (child processes)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_init_module 175
/* asm-generic/errno-base.h */
#define ENOEXEC 8
/* linux/capability.h */
#define CAP_VERSION_3 0x20080522
#define CAP_SYS_MODULE 16

struct cap_header {
    unsigned version;
    int pid;
};
struct cap_data {
    unsigned effective, permitted, inheritable;
};

static __attribute__((unused)) i64 init_module(const void *image, u64 len, const char *params) {
    return syscall3(SYS_init_module, image, len, params);
}

/* Empty all three capability sets (any user may do that; root then fails
 * every privilege check too) and confirm with capget that nothing is
 * left: 1 when the drop worked. */
static __attribute__((unused)) int drop_caps(void) {
    struct cap_header h = { CAP_VERSION_3, 0 };
    struct cap_data d[2];
    memset(d, 0, sizeof d);
    if (syscall2(SYS_capset, &h, d) != 0) return 0;
    h.version = CAP_VERSION_3;
    h.pid = 0;
    memset(d, 0xff, sizeof d);
    if (syscall2(SYS_capget, &h, d) != 0) return 0;
    return d[0].effective == 0 && d[1].effective == 0 && d[0].permitted == 0 && d[1].permitted == 0;
}

static __attribute__((unused)) void *map_pages(u64 n, int prot) {
    i64 r = syscall6(SYS_mmap, 0, n * 4096, prot, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    return r < 0 ? (void *)0 : (void *)r;
}

/* A buffer that starts like an x86-64 ELF relocatable object (the start of
 * Elf64_Ehdr from linux/elf.h: magic, ELFCLASS64, ELFDATA2LSB, EV_CURRENT,
 * e_type ET_REL 1, e_machine EM_X86_64 62); the rest is zero. */
static __attribute__((unused)) unsigned char elf_image[8192];
static __attribute__((unused)) void fill_elf(void) {
    memset(elf_image, 0, sizeof elf_image);
    elf_image[0] = 0x7f;
    elf_image[1] = 'E';
    elf_image[2] = 'L';
    elf_image[3] = 'F';
    elf_image[4] = 2;
    elf_image[5] = 1;
    elf_image[6] = 1;
    elf_image[16] = 1;
    elf_image[18] = 62;
}
static int set_effective(unsigned lo, unsigned hi) {
    struct cap_header h = { CAP_VERSION_3, 0 };
    struct cap_data d[2];
    memset(d, 0, sizeof d);
    syscall2(SYS_capget, &h, d);
    d[0].effective = lo & d[0].permitted;
    d[1].effective = hi & d[1].permitted;
    h.version = CAP_VERSION_3;
    h.pid = 0;
    return (int)syscall2(SYS_capset, &h, d);
}

static int child(unsigned lo, unsigned hi, i64 want) {
    i64 pid = fork();
    if (pid == 0) {
        if (set_effective(lo, hi) != 0) exit_group(1);
        if (init_module(0, 0, 0) != want) exit_group(2);
        if (init_module(0, 63, "") != want) exit_group(3);
        exit_group(0);
    }
    return wait_exit(pid);
}

int test_main(void) {
    i64 r = init_module(0, 0, 0);
    check("root_short_enoexec", r == -ENOEXEC, r);
    if (r == -EPERM) return finish();
    int code = child(1U << CAP_SYS_MODULE, 0, -ENOEXEC);
    check("only_sys_module_enoexec", code == 0, code);
    code = child(~(1U << CAP_SYS_MODULE), 0xffffffffU, -EPERM);
    check("all_but_sys_module_eperm", code == 0, code);
    code = child(0, 0, -EPERM);
    check("none_eperm", code == 0, code);
    r = init_module(0, 0, 0);
    check("parent_still_enoexec", r == -ENOEXEC, r);
    return finish();
}
