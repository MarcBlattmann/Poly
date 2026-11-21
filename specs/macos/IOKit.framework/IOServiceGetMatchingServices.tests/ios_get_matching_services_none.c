/* poly-mac: iosurface
 * IOServiceGetMatchingServices of a class nobody has, an empty class name: returns 0; the iterator it stores is NULL (0) there (no objects); for a NULL matching
 * dictionary it returns 0xe00002c2 and stores nothing. */
#include "poly_miosurface.h"
int test_main(void) {
    io_iterator_t it = 0x7777;
    kern_return_t kr = IOServiceGetMatchingServices(kIOMainPortDefault, IOServiceMatching("NoSuchPolyClass"), &it);
    check("a class that does not exist: returns 0", kr == KERN_SUCCESS, kr);
    value("the iterator it stores (0 means none)", it == 0x7777 ? -1 : (it != 0));
    if (it && it != 0x7777) check("an iterator that has no objects", IOIteratorNext(it) == 0, 0);
    it = 0x7777;
    kr = IOServiceGetMatchingServices(kIOMainPortDefault, IOServiceMatching(""), &it);
    check("the empty class name: returns 0", kr == KERN_SUCCESS, kr);
    it = 0x7777;
    kr = IOServiceGetMatchingServices(kIOMainPortDefault, NULL, &it);
    check("a NULL dictionary: 0xe00002c2", kr == (kern_return_t)0xe00002c2, kr);
    check("and nothing is stored", it == 0x7777, it);
    return finish();
}
