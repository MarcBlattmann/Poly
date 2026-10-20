/* execve replaces the address space: mappings of the old program are
 * gone, static data starts from its initial values again, and the new
 * program gets a fresh stack with its arguments. */
#include "a_sys.h"

#define FIXED 0x200000000L

static int initialised = 1234;
static int zeroed;

static int mapped(i64 a) {
    i64 r = syscall6(SYS_mmap, a, A_PAGE, PROT_NONE, MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED_NOREPLACE, -1, 0);
    if (r == a) {
        syscall2(SYS_munmap, a, A_PAGE);
        return 0;
    }
    return r == -EEXIST;
}

static int phase2(void) {
    passed = (int)a_atou(a_argv()[2]);
    check("old_mapping_gone", !mapped(FIXED), 0);
    expect("static_data_initial_again", initialised, 1234);
    expect("bss_zero_again", zeroed, 0);
    /* The stack holds argc, argv and envp as the psABI lays them out. */
    char **argv = a_argv();
    check("argv_on_new_stack", str_eq(argv[0], "m") && str_eq(argv[1], "2"), 0);
    check("stack_pointer_aligned", ((u64)poly_initial_sp & 15) == 0, (i64)poly_initial_sp);
    return finish();
}

int test_main(void) {
    if (a_argc() > 1 && str_eq(a_argv()[1], "2")) return phase2();
    i64 m = syscall6(SYS_mmap, FIXED, A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED_NOREPLACE,
                     -1, 0);
    expect("map_fixed", m, FIXED);
    *(volatile char *)m = 'x';
    initialised = 1;
    zeroed = 7;
    check("mapping_present", mapped(FIXED), 0);
    char count[24];
    a_utoa(count, (u64)passed + 1);
    char *argv[] = {"m", "2", count, 0};
    char *envp[] = {0};
    /* Failures so far would be lost across the execve: stop here. */
    if (failed) return finish();
    put("ok exec_now\n");
    i64 r = syscall3(SYS_execve, a_self_path(), argv, envp);
    expect("execve_returned", r, 0);
    return finish();
}
