/* dlopen(3) and friends on Poly (roadmap Phase 4, ADR 0049). Built at test
 * time with the host's C compiler, linked with DT_RUNPATH
 * "$ORIGIN/../lib/poly", so the dynamic linker finds libpolyplugin.so in
 * /lib/poly next to /bin (ld.so(8): $ORIGIN is the directory of the
 * program). The plugin needs libm, which comes with it.
 *
 * Prints "dyn-dlopen: ok" and exits 0; on a failed check it prints what
 * failed and exits 1. Written for Poly from the man pages. */

#define _GNU_SOURCE
#include <dlfcn.h>
#include <math.h>
#include <stdio.h>
#include <string.h>

static int failed;

static void check(int ok, const char *what) {
    if (!ok) {
        printf("dyn-dlopen: FAIL %s\n", what);
        failed = 1;
    }
}

int main(void) {
    void *h = dlopen("libpolyplugin.so", RTLD_NOW | RTLD_LOCAL);
    if (h == NULL) {
        printf("dyn-dlopen: FAIL dlopen: %s\n", dlerror());
        return 1;
    }
    int (*add)(int, int) = (int (*)(int, int))dlsym(h, "poly_plugin_add");
    double (*root)(double) = (double (*)(double))dlsym(h, "poly_plugin_cbrt");
    const char *(*name)(void) = (const char *(*)(void))dlsym(h, "poly_plugin_name");
    int *constructed = (int *)dlsym(h, "poly_plugin_constructed");
    check(add != NULL && root != NULL && name != NULL && constructed != NULL, "dlsym finds the exports");
    if (failed) {
        return 1;
    }
    check(add(40, 2) == 42, "calling a function of the plugin");
    check(fabs(root(74088.0) - 42.0) < 1e-9, "the plugin calls into libm");
    check(*constructed == 1, "the plugin's constructor ran once");
    check(strcmp(name(), "libpolyplugin") == 0, "the plugin's read-only data");
    dlerror();
    check(dlsym(h, "poly_plugin_missing") == NULL && dlerror() != NULL, "dlsym of a missing symbol fails");
    Dl_info info;
    check(dladdr((void *)add, &info) != 0 && info.dli_fname != NULL && strstr(info.dli_fname, "libpolyplugin.so"),
          "dladdr names the plugin");
    void *again = dlopen("libpolyplugin.so", RTLD_NOW | RTLD_NOLOAD);
    check(again == h, "RTLD_NOLOAD finds the loaded plugin");
    if (again != NULL) {
        dlclose(again);
    }
    check(dlopen("libpoly-not-there.so", RTLD_NOW) == NULL && dlerror() != NULL, "dlopen of a missing library fails");
    check(dlclose(h) == 0, "dlclose");
    if (failed) {
        return 1;
    }
    printf("dyn-dlopen: ok\n");
    return 0;
}
