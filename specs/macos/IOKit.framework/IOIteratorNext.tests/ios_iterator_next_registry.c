/* poly-mac: iosurface
 * IORegistryEntryCreateIterator(entry, kIOServicePlane, 0, &it) gives the children of an entry (the platform expert has some); with kIORegistryIterateRecursively the whole
 * subtree (more than 100 entries); IORegistryCreateIterator(kIOMainPortDefault, plane, options, &it) the whole registry; a plane that does not exist gives an error (0xe00002c0) from
 * IORegistryEntryGetParentEntry; the parent of the platform expert in the service plane exists (the root). */
#include "poly_miosurface.h"
int test_main(void) {
    io_service_t pe = IOServiceGetMatchingService(kIOMainPortDefault, IOServiceMatching("IOPlatformExpertDevice"));
    io_iterator_t it = 0;
    kern_return_t kr = IORegistryEntryCreateIterator(pe, kIOServicePlane, 0, &it);
    check("the children iterator: returns 0", kr == KERN_SUCCESS, kr);
    int children = ios_drain(it);
    check("the platform expert has children", children > 0, children);
    it = 0;
    kr = IORegistryEntryCreateIterator(pe, kIOServicePlane, kIORegistryIterateRecursively, &it);
    int all = ios_drain(it);
    check("recursively: more than 100 entries", kr == KERN_SUCCESS && all > 100, all);
    it = 0;
    kr = IORegistryCreateIterator(kIOMainPortDefault, kIOServicePlane, kIORegistryIterateRecursively, &it);
    int whole = ios_drain(it);
    check("the whole registry has at least as many", kr == KERN_SUCCESS && whole >= all, whole);
    io_registry_entry_t parent = 0;
    check("the parent in the service plane exists", IORegistryEntryGetParentEntry(pe, kIOServicePlane, &parent) == KERN_SUCCESS && parent != 0, 0);
    kern_return_t bad = IORegistryEntryGetParentEntry(pe, "IOPlane-NoSuchPoly", &parent);
    check("a plane that does not exist: an error", bad != KERN_SUCCESS, bad);
    value("the error for a plane that does not exist", bad);
    return finish();
}
