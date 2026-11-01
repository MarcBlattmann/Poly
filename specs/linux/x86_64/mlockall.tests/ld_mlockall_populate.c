/* What mlockall does to mappings, seen with mincore: MCL_FUTURE makes
 * every later mmap locked and populated at once (fresh anonymous pages
 * resident right after mmap); MCL_FUTURE|MCL_ONFAULT locks later
 * mappings without populating them; MCL_CURRENT populates every existing
 * mapping; MCL_CURRENT|MCL_ONFAULT locks them without populating; a call
 * with MCL_CURRENT but without MCL_FUTURE ends an earlier MCL_FUTURE
 * (later mappings are not locked); munlockall ends it too. A PROT_NONE
 * mapping does not make MCL_CURRENT fail.
 */
#include "a_sys.h"

#define MCL_CURRENT 1
#define MCL_FUTURE 2
#define MCL_ONFAULT 4

static unsigned char vec[16];
static i64 resident(char *p, int pages) {
    i64 r = syscall3(SYS_mincore, p, (u64)pages * A_PAGE, vec);
    if (r < 0) return r;
    i64 n = 0;
    for (int i = 0; i < pages; i++) n += vec[i] & 1;
    return n;
}
static char *map(int pages, int prot) {
    return (char *)syscall6(SYS_mmap, 0, (u64)pages * A_PAGE, prot, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
}

int test_main(void) {
    char *before = map(4, PROT_READ | PROT_WRITE);
    expect("unlocked_mmap_not_resident", resident(before, 4), 0);

    expect("future", syscall1(SYS_mlockall, MCL_FUTURE), 0);
    expect("future_leaves_existing", resident(before, 4), 0);
    char *a = map(4, PROT_READ | PROT_WRITE);
    expect("future_populates_new", resident(a, 4), 4);

    expect("future_onfault", syscall1(SYS_mlockall, MCL_FUTURE | MCL_ONFAULT), 0);
    char *b = map(4, PROT_READ | PROT_WRITE);
    expect("future_onfault_new_not_resident", resident(b, 4), 0);
    b[A_PAGE] = 1;
    expect("future_onfault_touched", resident(b, 4), 1);

    expect("current_onfault", syscall1(SYS_mlockall, MCL_CURRENT | MCL_ONFAULT), 0);
    expect("current_onfault_existing_not_populated", resident(before, 4), 0);
    char *c = map(4, PROT_READ | PROT_WRITE);
    expect("current_onfault_ended_future", resident(c, 4), 0);

    expect("current", syscall1(SYS_mlockall, MCL_CURRENT), 0);
    expect("current_populates_existing", resident(before, 4), 4);
    expect("current_populates_b", resident(b, 4), 4);
    expect("current_populates_c", resident(c, 4), 4);
    char *d = map(4, PROT_READ | PROT_WRITE);
    expect("current_alone_no_future", resident(d, 4), 0);

    expect("munlockall", syscall0(SYS_munlockall), 0);
    expect("current_future", syscall1(SYS_mlockall, MCL_CURRENT | MCL_FUTURE), 0);
    expect("current_future_existing", resident(d, 4), 4);
    char *e = map(4, PROT_READ | PROT_WRITE);
    expect("current_future_new", resident(e, 4), 4);
    expect("munlockall_ends_future", syscall0(SYS_munlockall), 0);
    char *f = map(4, PROT_READ | PROT_WRITE);
    expect("after_munlockall_new_not_resident", resident(f, 4), 0);

    char *n = map(2, PROT_NONE);
    (void)n;
    expect("current_with_prot_none_mapping", syscall1(SYS_mlockall, MCL_CURRENT), 0);
    expect("f_populated", resident(f, 4), 4);
    expect("munlockall_end", syscall0(SYS_munlockall), 0);
    return finish();
}
