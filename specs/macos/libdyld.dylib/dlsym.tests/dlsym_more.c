/* dlsym of data and of symbols in a framework. */
/* poly-mac: libsystem */
#include "poly_mlibc.h"
#include <dlfcn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

int test_main(void) {
    void *sys = dlopen("/usr/lib/libSystem.B.dylib", RTLD_NOW);
    void *p = dlsym(sys, "environ");
    value("environ via libSystem handle", p != NULL);
    void *cf = dlopen("/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation", RTLD_NOW);
    check("CFStringGetLength in CoreFoundation", cf && dlsym(cf, "CFStringGetLength") != NULL, 0);
    check("a data symbol: kCFAllocatorDefault", dlsym(cf, "kCFAllocatorDefault") != NULL, 0);
    check("a symbol of another library is not found through it: zlibVersion", dlsym(cf, "no_cf_symbol") == NULL, 0);
    dlerror();
    check("the same symbol twice gives the same address", dlsym(cf, "CFRetain") == dlsym(cf, "CFRetain"), 0);
    check("the address of malloc through three routes agrees", dlsym(RTLD_DEFAULT, "malloc") == dlsym(sys, "malloc") && dlsym(sys, "malloc") == (void *)malloc, 0);
    return finish();
}
