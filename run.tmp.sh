cargo xtask coverage claim linux/syscall/x86_64/sysfs --agent impl-linux-f 2>&1 | tail -1
cargo test -p poly-personality-linux -p poly-vm 2>&1 | grep -E '^error|test result|FAILED|panicked' | head -20
S=specs/linux/x86_64
T=""; D=""
for e in ptrace perf_event_open modify_ldt uprobe uretprobe; do D="$D --tests-dir $S/$e.tests"; for f in $S/$e.tests/*.c; do T="$T $(basename $f .c)"; done; done
cargo xtask pipeline polyrun $T $D --build-kernel --print > target/polyrun.log 2>&1
echo "polyrun exit $?"
grep -E '^error|^\S+: (pass|fail|exit)|not ok|^==|exit status|killed|POLY-INIT' target/polyrun.log | head -80
