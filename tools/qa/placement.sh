#!/bin/bash
# chrome-qa experiment (runs inside polyd): back up Chrome's disk, then set the profile's saved
# window placement to a normal on-screen window. Restore with: cp target/qa-disk-backup.img target/poly-chrome-try-disk.img
set -e
d=/repo/target/poly-chrome-try-disk.img
p=/poly/nt/c/poly-chrome-profile/Default/Preferences
[ -f /repo/target/qa-disk-backup.img ] || cp $d /repo/target/qa-disk-backup.img
e2fsck -fy $d >/tmp/fsck.log 2>&1 || true; tail -2 /tmp/fsck.log
debugfs -R "dump $p /tmp/Preferences" $d 2>/dev/null
python3 - <<'PY'
import json
p = json.load(open('/tmp/Preferences'))
print('before', p['browser']['window_placement'])
p['browser']['window_placement'].update({'maximized': False, 'left': 16, 'top': 34, 'right': 1053, 'bottom': 783})
print('after ', p['browser']['window_placement'])
json.dump(p, open('/tmp/Preferences.new', 'w'), separators=(',', ':'))
PY
debugfs -w -R "rm $p" $d
debugfs -w -R "write /tmp/Preferences.new $p" $d
debugfs -R "dump $p /tmp/check" $d 2>/dev/null; python3 -c "import json;print('on disk', json.load(open('/tmp/check'))['browser']['window_placement'])"
