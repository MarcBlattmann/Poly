// Declarations of the msvcp140.dll C threading functions (extern "C"), from the layout and names of the STL's
// xthreads.h / xtimec.h: types come from <xthreads.h> (_Thrd_t, _Mtx_t, _Cnd_t, _Thrd_result).
#pragma once
#include <xthreads.h>

struct poly_xtime {
    long long sec;
    long nsec;
};
typedef int(__cdecl* poly_thrd_start)(void*);

extern "C" {
__declspec(dllimport) _Thrd_result __cdecl _Mtx_init(_Mtx_t*, int);
__declspec(dllimport) void __cdecl _Mtx_destroy(_Mtx_t);
__declspec(dllimport) _Thrd_result __cdecl _Cnd_init(_Cnd_t*);
__declspec(dllimport) void __cdecl _Cnd_destroy(_Cnd_t);
__declspec(dllimport) _Thrd_result __cdecl _Thrd_create(_Thrd_t*, poly_thrd_start, void*);
__declspec(dllimport) _Thrd_t __cdecl _Thrd_current();
__declspec(dllimport) int __cdecl _Thrd_equal(_Thrd_t, _Thrd_t);
__declspec(dllimport) void __cdecl _Thrd_sleep(const poly_xtime*);
__declspec(dllimport) unsigned long __stdcall GetCurrentThreadId();
__declspec(dllimport) void __stdcall Sleep(poly_u32);
}
