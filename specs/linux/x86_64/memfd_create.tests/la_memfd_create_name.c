/* The name: up to 249 bytes (NAME_MAX 255 minus the "memfd:" prefix)
 * are accepted, 250 or more are EINVAL. Any bytes are allowed, also '/'
 * and the empty name. The name only labels the file: the descriptor's
 * /proc/self/fd link reads "/memfd:<name> (deleted)". An unreadable name
 * (NULL, a low address, a string running into an unmapped page before
 * its terminator) is EFAULT; a too long name that runs into an unmapped
 * page after 250 readable bytes is EINVAL.
 */
#include "a_sys.h"

#define SYS_memfd_create 319

static i64 memfd(const char *name, u64 flags) { return syscall2(SYS_memfd_create, name, flags); }

static i64 try_name(const char *name) {
    i64 fd = memfd(name, 0);
    if (fd >= 0) a_close(fd);
    return fd;
}

static char long_name[400];

/* Read the link of descriptor fd into out; returns its length. */
static i64 fd_link(i64 fd, char *out, size_t size) {
    char path[40];
    char *p = str_cpy(path, "/proc/self/fd/");
    a_utoa(p, (u64)fd);
    i64 n = syscall4(SYS_readlinkat, AT_FDCWD, path, out, size - 1);
    out[n > 0 ? n : 0] = 0;
    return n;
}

int test_main(void) {
    memset(long_name, 'n', sizeof long_name);
    long_name[249] = 0;
    expect("len_249_ok", try_name(long_name), 3);
    long_name[249] = 'n';
    long_name[250] = 0;
    expect("len_250_einval", try_name(long_name), -EINVAL);
    long_name[250] = 'n';
    long_name[300] = 0;
    expect("len_300_einval", try_name(long_name), -EINVAL);
    long_name[248] = 0;
    expect("len_248_ok", try_name(long_name), 3);

    expect("empty_ok", try_name(""), 3);
    expect("slash_ok", try_name("a/b/c"), 3);
    expect("spaces_ok", try_name("with space"), 3);

    expect("null_efault", try_name(0), -EFAULT);
    expect("low_address_efault", try_name((const char *)8), -EFAULT);

    /* A string at the end of a page followed by an unmapped page. */
    char *two = (char *)syscall6(SYS_mmap, 0, 2 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    syscall2(SYS_munmap, two + A_PAGE, A_PAGE);
    char *end = two + A_PAGE;
    memset(two, 'x', A_PAGE);
    expect("unterminated_short_efault", try_name(end - 10), -EFAULT);
    expect("unterminated_long_einval", try_name(end - 300), -EINVAL);
    end[-1] = 0;
    expect("terminated_at_page_end_ok", try_name(end - 10), 3);

    char link[300];
    i64 fd = memfd("label", 0);
    i64 n = fd_link(fd, link, sizeof link);
    expect("link_len", n, 22);
    check("link_text", str_eq(link, "/memfd:label (deleted)"), n);
    a_close(fd);

    fd = memfd("", 0);
    fd_link(fd, link, sizeof link);
    check("empty_link_text", str_eq(link, "/memfd: (deleted)"), 0);
    a_close(fd);

    long_name[248] = 'n';
    long_name[249] = 0;
    fd = memfd(long_name, 0);
    n = fd_link(fd, link, sizeof link);
    expect("long_link_len", n, 1 + 6 + 249 + 10);
    a_close(fd);
    return finish();
}
