#!/bin/bash
# Host side: tools/app-survey/all.sh [NAME...]  runs the apps of apps.txt (or only NAMEs) one after the other.
cd "$(dirname "$0")/../.." || exit 1
POLYD=C:/Users/Marc.Blattmann/poly-dev/polyd
want=" $* "
old= prev=
while IFS='|' read -r name pkgs secs line; do
  case $name in ''|'#'*) continue;; esac
  if [ $# -gt 0 ] && [[ "$want" != *" $name "* ]]; then continue; fi
  echo "$(date +%T) $name: install"
  rm -rf "survey-out/$name-install" "survey-out/$name-start"
  $POLYD "bash tools/app-survey/survey.sh install $name '$pkgs'" > "survey-out/$name.install.log" 2>&1 < /dev/null
  echo "$(date +%T) $name: install rc=$?"
  $POLYD "bash tools/app-survey/survey.sh launch $name start $secs '$line'" > "survey-out/$name.start.log" 2>&1 < /dev/null
  echo "$(date +%T) $name: start rc=$?"
  bash tools/app-survey/summarize.sh "$name" > "survey-out/$name.summary.txt" 2>&1 < /dev/null
  # the apps' disks are big: keep the last two (for reruns), drop the one before
  [ -n "$old" ] && $POLYD "bash tools/app-survey/survey.sh drop $old" > /dev/null 2>&1 < /dev/null
  old=$prev; prev=$name
  echo "$(date +%T) $name: done"
done < tools/app-survey/apps.txt
echo ALL-DONE
