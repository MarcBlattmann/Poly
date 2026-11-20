/* poly-mac: foundation */
/* lastPathComponent: the part after the last slash, ignoring trailing slashes;
 * '/' for the root; the empty string for the empty string; a string without
 * slash is its own last component. */
#include "poly_mfn.h"

#define L(path, want) check(path, ns_is([@path lastPathComponent], want), 0)

int test_main(void) {
    L("/a/b/c.txt", "c.txt");
    L("/a/b/", "b");
    L("/", "/");
    L("", "");
    L("file", "file");
    L("//a//", "a");
    L("a/b", "b");
    L("/a", "a");
    L("/café/xé", "x\xc3\xa9");
    return finish();
}
