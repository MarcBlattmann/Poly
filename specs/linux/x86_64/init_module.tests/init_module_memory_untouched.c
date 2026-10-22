/* A failed init_module leaves the caller's memory alone: the image and
 * parameter buffers keep their bytes, an image and parameters in
 * read-only memory work (EPERM, no signal), and the memory stays usable
 * afterwards.
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
int test_main(void) {
    int dropped = drop_caps();
    check("caps_dropped", dropped, dropped);
    if (!dropped) return finish();
    fill_elf();
    for (int i = 64; i < (int)sizeof elf_image; i++) elf_image[i] = (unsigned char)(i * 7);

    static unsigned char copy[sizeof elf_image];
    memcpy(copy, elf_image, sizeof elf_image);
    char params[32] = "alpha=1 beta=2,3 gamma";
    char pcopy[32];
    memcpy(pcopy, params, sizeof params);

    i64 r = init_module(elf_image, sizeof elf_image, params);
    check("eperm", r == -EPERM, r);
    check("image_unchanged", mem_eq(elf_image, copy, sizeof elf_image), 0);
    check("params_unchanged", mem_eq(params, pcopy, sizeof params), 0);

    unsigned char *ro = map_pages(2, PROT_READ | PROT_WRITE);
    check("mmap", ro != 0, 0);
    if (!ro) return finish();
    memcpy(ro, elf_image, 4096);
    memcpy(ro + 4096, params, sizeof params);
    syscall3(SYS_mprotect, ro, 2 * 4096, PROT_READ);
    r = init_module(ro, 4096, (const char *)(ro + 4096));
    check("read_only_eperm", r == -EPERM, r);
    check("read_only_image_unchanged", mem_eq(ro, elf_image, 4096), 0);
    check("read_only_params_unchanged", mem_eq(ro + 4096, params, sizeof params), 0);
    r = syscall3(SYS_mprotect, ro, 2 * 4096, PROT_READ | PROT_WRITE);
    check("mprotect_back", r == 0, r);
    ro[0] = 1;
    check("still_writable", ro[0] == 1, ro[0]);
    return finish();
}
