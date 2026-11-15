// poly-cxx: compiler=msvc
// msvcp140.dll std::_Lockit (?0_Lockit@std@@QEAA@H@Z and ?1_Lockit@std@@QEAA@XZ: constructor from a lock number, destructor):
// the lock is held for the object's lifetime; it is recursive within a thread; another thread's _Lockit of the same
// number waits, a different number does not; the object is 4 bytes (the lock number).
#include "poly_wtest.h"
#include <new>
#include <xlocale>

extern "C" {
__declspec(dllimport) void* __stdcall CreateThread(void*, unsigned long long, unsigned long (__stdcall*)(void*), void*, poly_u32, poly_u32*);
__declspec(dllimport) poly_u32 __stdcall WaitForSingleObject(void*, poly_u32);
__declspec(dllimport) void __stdcall Sleep(poly_u32);
__declspec(dllimport) long long __cdecl _Xtime_get_ticks();
}

struct Shared {
    int number;
    volatile int entered;
    volatile long long waited_ms;
    volatile int go;
    volatile int ready;
};

static unsigned long __stdcall contender(void* p) {
    Shared* s = (Shared*)p;
    s->ready = 1;
    while (!s->go) Sleep(1);
    long long t0 = _Xtime_get_ticks();
    {
        std::_Lockit lock(s->number);
        s->waited_ms = (_Xtime_get_ticks() - t0) / 10000;
        s->entered = 1;
    }
    return 0;
}

static void test(poly::Test& t) {
    t.expect("sizeof(std::_Lockit)", (long long)sizeof(std::_Lockit), 4);
    {
        std::_Lockit a(0);
        std::_Lockit b(0);
        std::_Lockit c(1);
        t.pass("the same lock twice in one thread (recursive), and another number");
    }
    alignas(8) char storage[16];
    {
        // A different number does not wait for ours.
        Shared s = {1, 0, 0, 0, 0};
        void* h = CreateThread(0, 0, contender, &s, 0, 0);
        while (!s.ready) Sleep(1);
        std::_Lockit* holder = new (storage) std::_Lockit(0);
        s.go = 1;
        for (int i = 0; i < 100 && !s.entered; i++) Sleep(10);
        t.expect("another thread's _Lockit of a different number does not wait", s.entered, 1);
        holder->~_Lockit();
        t.expect("the thread ends", (int)WaitForSingleObject(h, 5000), 0);
    }
    {
        // The same number does.
        Shared s = {0, 0, 0, 0, 0};
        void* h = CreateThread(0, 0, contender, &s, 0, 0);
        while (!s.ready) Sleep(1);
        std::_Lockit* holder = new (storage) std::_Lockit(0);
        s.go = 1;
        Sleep(150);
        t.expect("another thread's _Lockit of the same number waits", s.entered, 0);
        holder->~_Lockit();
        t.expect("it enters after the holder is destroyed", (int)WaitForSingleObject(h, 5000), 0);
        t.expect("it entered", s.entered, 1);
        t.check("it waited about the 150 ms", s.waited_ms >= 100 && s.waited_ms < 3000, s.waited_ms);
    }
}
POLY_TEST_MAIN(test)
