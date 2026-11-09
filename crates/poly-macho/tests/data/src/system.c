/* A stand-in for the system library, written here: classic lazy binding
   needs a dylib that exports the binder symbol, and we do not use Apple's. */
/* The linker looks for "dyld_stub_binder"; Mach-O C symbols carry a
   leading underscore, so ask for the name without one with an asm label. */
void binder(void) __asm__("dyld_stub_binder");
void binder(void) {}
