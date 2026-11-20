/* Shared by the exception tests (ADR 0238). */
#include <mach/mach.h>
#include <mach/mach_types.h>
#define MACH(n) (0x1000000L | (n))
#define BSD(n) (0x2000000L | (n))
long poly_sys(long n, long a, long b, long c, long d, long e, long f);
void check(const char *name, int ok, long got);
void value(const char *name, long v);
int finish(void);
__attribute__((noreturn)) void poly_exit(int code);
int spawn(const char *path, char *const argv[]);
int wait_child(int pid);
#define W_EXITED(s) (((s) & 0177) == 0)
#define W_EXITSTATUS(s) (((s) >> 8) & 0xff)
