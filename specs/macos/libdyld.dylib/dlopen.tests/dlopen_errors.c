/* dlopen failures: NULL, and dlerror gives one string that is cleared by
 * being read. Only the stable parts of the message are checked: it starts
 * with "dlopen(<path>, <mode in hex>): " and lists what was tried. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <dlfcn.h>
#include <fcntl.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

static int starts(const char *s, const char *p) { return s && strncmp(s, p, strlen(p)) == 0; }
static int has(const char *s, const char *p) { return s && strstr(s, p) != NULL; }

int test_main(void) {
    check("dlerror before any failure is NULL", dlerror() == NULL, 0);
    void *h = dlopen("/nonexistent/libnope.dylib", RTLD_NOW);
    check("a missing file: NULL", h == NULL, 0);
    char *e = dlerror();
    check("the message starts with dlopen(path, mode):", starts(e, "dlopen(/nonexistent/libnope.dylib, 0x0002): "), 0);
    check("it says what was tried", has(e, "tried: '/nonexistent/libnope.dylib' (no such file)"), 0);
    check("read once, it is gone: dlerror is NULL again", dlerror() == NULL, 0);

    h = dlopen("/nonexistent/libnope.dylib", RTLD_LAZY | RTLD_LOCAL);
    e = dlerror();
    check("the mode is shown in hex: RTLD_LAZY|RTLD_LOCAL (1|4) is 0x0005", starts(e, "dlopen(/nonexistent/libnope.dylib, 0x0005): "), 0);
    h = dlopen("/nonexistent/libnope.dylib", RTLD_NOW | RTLD_GLOBAL);
    e = dlerror();
    check("RTLD_NOW|RTLD_GLOBAL (2|8) is 0x000A", starts(e, "dlopen(/nonexistent/libnope.dylib, 0x000A): "), 0);

    h = dlopen("@rpath/libx.dylib", RTLD_NOW);
    e = dlerror();
    check("@rpath with no LC_RPATH in the program", h == NULL && starts(e, "dlopen(@rpath/libx.dylib, 0x0002): ") && has(e, "no LC_RPATH's found"), 0);

    h = dlopen("@executable_path/libx.dylib", RTLD_NOW);
    e = dlerror();
    check("@executable_path names the program's directory", h == NULL && has(e, "/libx.dylib' (no such file)") && has(e, "tried: '/"), 0);

    h = dlopen("libnope.dylib", RTLD_NOW);
    e = dlerror();
    check("a bare name is tried as given and under /usr/lib", h == NULL && has(e, "'libnope.dylib' (no such file)") && has(e, "'/usr/lib/libnope.dylib' (no such file, not in dyld cache)"), 0);

    int fd = open("notmacho.dylib", O_WRONLY | O_CREAT, 0644);
    write(fd, "this is plainly not a Mach-O file, only text, but more than a few bytes long.........................................", 110);
    close(fd);
    h = dlopen("./notmacho.dylib", RTLD_NOW);
    e = dlerror();
    check("a file that is not Mach-O", h == NULL && has(e, "'./notmacho.dylib' (slice is not valid mach-o file)"), 0);

    fd = open("empty.dylib", O_WRONLY | O_CREAT, 0644);
    close(fd);
    h = dlopen("./empty.dylib", RTLD_NOW);
    e = dlerror();
    check("an empty file: the reason is empty parentheses", h == NULL && has(e, "'./empty.dylib' ()"), 0);

    mkdir("adir", 0755);
    h = dlopen("./adir", RTLD_NOW);
    e = dlerror();
    check("a directory: not a file", h == NULL && has(e, "'./adir' (not a file)"), 0);
    h = dlopen("", RTLD_NOW);
    e = dlerror();
    check("an empty path is a failure too", h == NULL && starts(e, "dlopen(, 0x0002): "), 0);
    return finish();
}
