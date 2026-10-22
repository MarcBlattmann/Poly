/* Several processes calling mkdir on the same new name at the same
 * time: exactly one gets 0, every other one EEXIST, and the result is
 * one directory. Different names made at the same time all succeed.
 * poly-diff: syscalls=off (which child wins depends on scheduling)
 */
#include "a_sys.h"

#define D "/tmp/c_mkdir_race"
#define N 6
#define ROUNDS 20

static i64 mk(const char *p, i64 m) { return syscall2(SYS_mkdir, p, m); }

int test_main(void) {
    volatile i32 *sh = a_shared();
    expect("setup", a_mkdir(D, 0755), 0);
    int all_one_winner = 1, all_eexist = 1, own_names_ok = 1;
    for (int r = 0; r < ROUNDS; r++) {
        sh[0] = 0;
        for (int i = 0; i < N; i++) sh[1 + i] = 99;
        for (int i = 0; i < N; i++) {
            i64 pid = syscall0(SYS_fork);
            if (pid == 0) {
                while (sh[0] == 0) a_relax();
                i64 res = mk(D "/same", 0755);
                char own[64];
                str_cpy(own, D "/own_");
                size_t l = str_len(own);
                own[l] = (char)('a' + i);
                own[l + 1] = 0;
                i64 res2 = mk(own, 0755);
                sh[1 + i] = (i32)res;
                sh[1 + N + i] = (i32)res2;
                exit_group(0);
            }
        }
        sh[0] = 1;
        for (int i = 0; i < N; i++) syscall4(SYS_wait4, -1, 0, 0, 0);
        int winners = 0;
        for (int i = 0; i < N; i++) {
            if (sh[1 + i] == 0) winners++;
            else if (sh[1 + i] != -EEXIST) all_eexist = 0;
            if (sh[1 + N + i] != 0) own_names_ok = 0;
        }
        if (winners != 1) all_one_winner = 0;
        if (a_rmdir(D "/same") != 0) all_one_winner = 0;
        for (int i = 0; i < N; i++) {
            char own[64];
            str_cpy(own, D "/own_");
            size_t l = str_len(own);
            own[l] = (char)('a' + i);
            own[l + 1] = 0;
            if (a_rmdir(own) != 0) own_names_ok = 0;
        }
    }
    expect("exactly_one_winner_each_round", all_one_winner, 1);
    expect("losers_get_eexist", all_eexist, 1);
    expect("distinct_names_all_succeed", own_names_ok, 1);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
