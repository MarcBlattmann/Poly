/* A tiny program that imports from libtiny and has pointers to rebase. */
extern int lib_add(int, int);
extern int lib_sub(int, int);
extern int lib_value;
extern int lib_weak(void) __attribute__((weak_import));
extern int weakdep_fn(void) __attribute__((weak_import));

static int local_value = 5;
static int local_fn(int a, int b) { return a * b; }

/* Data with pointers: a bind (lib_add, lib_sub), two rebases (local_fn,
   &local_value) and one more bind (&lib_value). */
int (*fn_table[])(int, int) = {lib_add, lib_sub, local_fn};
int *value_ptrs[] = {&lib_value, &local_value};
const char *name = "poly";

__attribute__((weak)) int app_weak(void) { return 1; }

int main(void) {
    int r = fn_table[0](1, 2) + *value_ptrs[0] + lib_sub(5, 3);
    if (weakdep_fn) r += weakdep_fn();
    if (lib_weak) r += lib_weak();
    return r + app_weak();
}
