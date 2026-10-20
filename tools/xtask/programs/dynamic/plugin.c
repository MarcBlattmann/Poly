/* The shared object `dyn-dlopen` loads with dlopen(3) (roadmap Phase 4, ADR
 * 0049). It needs the maths library itself (DT_NEEDED libm.so.6), which the
 * dynamic linker loads with it, has a constructor that must run exactly
 * once, and exports a few functions and a variable for dlsym(3).
 * Written for Poly from the man pages. */

#include <math.h>

int poly_plugin_constructed;

__attribute__((constructor)) static void construct(void) {
    poly_plugin_constructed++;
}

int poly_plugin_add(int a, int b) {
    return a + b;
}

/* cbrt(3) always calls into libm. */
double poly_plugin_cbrt(double x) {
    return cbrt(x);
}

const char *poly_plugin_name(void) {
    return "libpolyplugin";
}
