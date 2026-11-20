/* poly-mac: foundation */
/* stringByAppendingPathComponent: puts exactly one slash between the two parts:
 * a trailing slash on the receiver or a leading one on the argument is not
 * doubled, an empty argument leaves the receiver (without its trailing slash),
 * an empty receiver gives the argument alone, a component with slashes is kept
 * whole and '..' is not resolved. */
#include "poly_mfn.h"

#define A(base, comp, want) check(base " + " comp, ns_is([@base stringByAppendingPathComponent:@comp], want), 0)

int test_main(void) {
    A("/usr/local", "bin", "/usr/local/bin");
    A("/usr/local/", "bin", "/usr/local/bin");
    A("/usr/local", "/bin", "/usr/local/bin");
    A("/usr/local", "", "/usr/local");
    A("", "bin", "bin");
    A("/usr/local", "../x", "/usr/local/../x");
    A("/usr/local", "a/b", "/usr/local/a/b");
    A("/a//", "b", "/a/b");
    A("rel", "x", "rel/x");
    A("/", "x", "/x");
    return finish();
}
