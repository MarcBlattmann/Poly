/* A tiny shared library: functions, data and a weak definition to export. */
int lib_value = 42;
int lib_values[4] = {1, 2, 3, 4};
int lib_add(int a, int b) { return a + b; }
int lib_sub(int a, int b) { return a - b; }
int lib_sum(int a, int b) { return lib_add(a, b) + lib_sub(a, b); }
__attribute__((weak)) int lib_weak(void) { return 7; }
