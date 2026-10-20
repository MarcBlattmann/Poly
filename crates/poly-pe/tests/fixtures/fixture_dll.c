/* Source of fixture.dll, a tiny freestanding DLL used by poly-pe's tests.
 * Built by build.sh with clang + lld-link; no C runtime, no Windows SDK. */

__declspec(dllexport) int fixture_add(int a, int b) { return a + b; }
__declspec(dllexport) int fixture_counter = 5;

/* An absolute pointer: gives the DLL a DIR64 base relocation. */
__declspec(dllexport) int *fixture_counter_ptr = &fixture_counter;

/* Exported by ordinal only (see fixture_dll.def). */
int fixture_hidden(void) { return 7; }

int _DllMainCRTStartup(void *module, unsigned long reason, void *reserved) {
    (void)module;
    (void)reason;
    (void)reserved;
    return 1;
}
