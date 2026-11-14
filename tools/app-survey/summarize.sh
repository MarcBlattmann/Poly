#!/bin/bash
# summarize.sh NAME [LABEL]: short digest of survey-out/NAME-install and NAME-LABEL (default start) for the survey table.
n=$1 l=${2:-start}
I=survey-out/$n-install/console.log
S=survey-out/$n-$l/console.log
echo "=== $n: install"
if [ -f "$I" ]; then
  grep -a "apt-exit-[0-9]*$\|^E: \|Err:" "$I" | grep -av "echo " | cut -c1-200 | sort -u | head -n 8
  grep -a "^    | ii \|^ii " "$I" | head -n 3 | cut -c1-110
  grep -a "/dev/vda " "$I" | tail -n 2 | cut -c1-80
else
  echo "(no install log)"
fi
echo "=== $n: $l"
if [ -f "$S" ]; then
  grep -a 'poly-compositor: window "' "$S" | cut -c1-160 | head -n 6
  echo "--- app log"
  awk '/^==APPLOG==/{f=1;next} /^==PS==/{f=0} f' "$S" | grep -av "^$" | cut -c1-220 | head -n 25
  echo "--- ps (poly / interesting)"
  awk '/^==PS==/{f=1;next} /^==END-1==/{f=0} f' "$S" | grep -av " ps -eo\| cut -c\| tail -n\|busybox init\|bash -l\|poly-compositor" | cut -c1-150 | head -n 12
  echo "--- kernel complaints"
  grep -a "not implemented\|killed by signal\|unsupported\|ENOSYS\|panic\|page fault" "$S" | sort | uniq -c | sort -rn | cut -c1-200 | head -n 12
  echo "--- processes that ended badly (poly.exits, non-zero status)"
  grep -a "pid [0-9]* (.*) \[.*exit status [1-9]\|signal [0-9]" "$S" | grep -av "grep\|dpkg\|stty\|\[sh -c\|test -\|ischroot" | cut -c1-200 | head -n 10
  echo "--- windows closed"
  grep -ac "window.*closed\|destroyed" "$S"
else
  echo "(no $l log)"
fi
ls survey-out/$n-$l/*.png 2>/dev/null | sed 's|.*/||' | tr '\n' ' '; echo
