#!/bin/bash
# chrome-qa: boot the desktop in polyd and follow target/qa/cmds-TAG from the host.
# usage (Git Bash, worktree root): tools/qa/run.sh TAG [win|linux] [extra desktop args...]
tag=$1; mode=${2:-win}; shift 2
q=target/qa; mkdir -p $q/shots-$tag; : > $q/cmds-$tag; : > $q/cmds-$tag.out
if [ "$mode" = win ]; then disk=--chrome-win; else disk=--chromium; fi
exec /c/Users/Marc.Blattmann/poly-dev/polyd "
cargo xtask desktop --release $disk --size=2560x1600 --scale=2 --qmp-port=4444 --compositor-stats $* </dev/null >/tmp/serial.log 2>&1 &
qp=\$!
( tail -F /tmp/serial.log 2>/dev/null > /host-repo/$q/serial-$tag.log ) &
tp=\$!
python3 /repo/tools/qa/drive.py 4444 /host-repo/$q/cmds-$tag /host-repo/$q/shots-$tag /tmp/serial.log
wait \$qp; sleep 2; kill \$tp
"
