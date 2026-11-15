#!/bin/bash
# Linux app survey driver (runs inside the dev container: polyd 'bash tools/app-survey/survey.sh ...').
#
#   survey.sh base
#       Make the base disk: Chromium's desktop disk with room for big apps,
#       Debian's packages configured and apt's lists fetched (once).
#   survey.sh install NAME "PKG PKG..." [EXTRA_APT_ARGS]
#       Fresh copy of the base disk, apt-get install, power off. The disk
#       stays in target/survey-NAME.img.
#   survey.sh run NAME LABEL STEP...
#       Boot target/survey-NAME.img (installed earlier) and run xtask steps
#       (run:LINE, shot:NAME, key:CHORD, type:TEXT, click:X,Y, sleep:N).
#       Screenshots and the console log go to survey-out/NAME-LABEL/.
#   survey.sh launch NAME LABEL SECONDS 'COMMAND LINE'
#       Boot, start COMMAND as user poly on the desktop, wait, screenshot,
#       print what the app said and which Linux processes ended how.
#   survey.sh drop NAME
#       Delete target/survey-NAME.img.
set -u
cd "$(dirname "$0")/../.." || exit 1
export POLY_ROOTDISK_SPARE_MB="${POLY_ROOTDISK_SPARE_MB:-5120}"
# Linux processes that end are logged. (poly.exits.trail=N adds the last N calls
# of every process that ends: only for a rerun of one app, it floods the console.)
export POLY_CHROMIUM_APPEND="${POLY_CHROMIUM_APPEND:-poly.exits}"
PDF='printf "%%PDF-1.1\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj\n3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 200]>>endobj\ntrailer<</Root 1 0 R>>\n" > /tmp/hello.pdf; chmod 666 /tmp/hello.pdf'
SVG='printf "<svg xmlns='"'"'http://www.w3.org/2000/svg'"'"' width='"'"'100'"'"' height='"'"'100'"'"'><rect width='"'"'80'"'"' height='"'"'80'"'"' fill='"'"'red'"'"'/></svg>" > /tmp/hello.svg; chmod 666 /tmp/hello.svg'
IMG=target/poly-chromium-disk.img
BASE=target/survey-base.img
OUT=survey-out

xt() { # xt LABEL STEP...  (boot the disk at $IMG, run the steps)
  local label=$1; shift
  mkdir -p "$OUT/$label"
  POLY_TOOLKITS_SHOTS="$PWD/$OUT/$label" POLY_CHROMIUM_LOG="$PWD/$OUT/$label/console.log" \
    cargo xtask chromium "$@" 2>&1 | grep --line-buffered -v '^\s*Compiling\|^\s*Finished' | tee "$OUT/$label/xtask.log"
  return "${PIPESTATUS[0]}"
}

cmd=${1:-}; shift || true
case $cmd in
base)
  xt base --fresh-disk \
    'run:dpkg --configure -a > /tmp/configure.log 2>&1; echo configure-exit-$?' \
    'run:tail -n 3 /tmp/configure.log' \
    'run:apt-get update > /tmp/update.log 2>&1; echo update-exit-$?' \
    'run:tail -n 3 /tmp/update.log; df -h / | tail -n 1' || exit 1
  cp --sparse=always "$IMG" "$BASE" && ls -ls "$BASE"
  ;;
install)
  name=$1 pkgs=$2 extra=${3:-}
  cp --sparse=always "$BASE" "$IMG" || exit 1
  xt "$name-install" \
    'run:df -h / | tail -n 1' \
    "run:(date +%s; DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends $extra $pkgs > /tmp/apt.log 2>&1; echo apt-exit-\$?; date +%s)" \
    'run:grep -E "^(E:|Err|W:)" /tmp/apt.log | head -n 20; tail -n 4 /tmp/apt.log; df -h / | tail -n 1' \
    "run:dpkg -l $pkgs | grep '^ii' | head -n 12 | cut -c1-110" \
    'run:dpkg --audit | head; echo audit-done'
  rc=$?
  mv "$IMG" "target/survey-$name.img"
  exit $rc
  ;;
run)
  name=$1 label=$2; shift 2
  cp --sparse=always "target/survey-$name.img" "$IMG" || exit 1
  xt "$name-$label" "$@"
  rc=$?
  mv "$IMG" "target/survey-$name.img"
  exit $rc
  ;;
launch)
  # launch NAME LABEL SECONDS 'COMMAND LINE' [STEP...]  (no STEPs: click, type, ctrl+o dialog, ctrl+q)
  name=$1 label=$2 secs=$3 line=$4; shift 4
  if [ $# -eq 0 ]; then
    set -- 'click:640,420' 'type:typed by poly 123' 'sleep:2' 'shot:typed' \
      'key:ctrl+o' 'sleep:3' 'shot:open-dialog' 'key:esc' 'sleep:1' 'key:ctrl+q' 'sleep:4' 'shot:after-close'
  fi
  cp --sparse=always "target/survey-$name.img" "$IMG" || exit 1
  rm -rf "$OUT/$name-$label"
  xt "$name-$label" \
    'key:esc' "send:$PDF" "send:$SVG" \
    "send:echo 'hello from poly' > /tmp/hello.txt; chmod 666 /tmp/hello.txt; . /etc/poly-desktop.env; (poly-as-user $line > /tmp/app.log 2>&1 &) " \
    "sleep:$secs" \
    "shot:start" \
    "$@" \
    'send:(echo ==APPLOG==; head -c 3000 /tmp/app.log; echo ==PS==; ps -eo pid,user,stat,etime,args | cut -c1-150 | tail -n 30; echo ==END-$((1))==)' \
    'expect:==END-1=='
  # (Not `run:` once an app is up: its desktop messages break the prompt detection.)
  rc=$?
  mv "$IMG" "target/survey-$name.img"
  exit $rc
  ;;
vscode)
  # vscode: the official .deb from Microsoft's download page, installed with apt from a throw-away local repository
  # served by the container to the guest at 10.0.2.2 (the guest has no curl/wget).
  mkdir -p /tmp/vsc && cd /tmp/vsc || exit 1
  curl -fL --retry 3 -o code.deb 'https://update.code.visualstudio.com/latest/linux-deb-x64/stable' || exit 1
  { dpkg-deb -f code.deb; echo "Filename: ./code.deb"; echo "Size: $(stat -c %s code.deb)"; echo "SHA256: $(sha256sum code.deb | cut -d' ' -f1)"; echo; } > Packages
  dpkg-deb -f code.deb Version | sed 's/^/vscode deb version: /'
  (python3 -m http.server 8765 --bind 0.0.0.0 >/tmp/vsc-http.log 2>&1 &)
  cd - >/dev/null || exit 1
  cp --sparse=always "$BASE" "$IMG" || exit 1
  rm -rf "$OUT/vscode-install"
  xt vscode-install \
    "run:echo 'deb [trusted=yes] http://10.0.2.2:8765/ ./' > /etc/apt/sources.list.d/vsc.list; apt-get update > /tmp/update2.log 2>&1; echo update-exit-\$?; tail -n 2 /tmp/update2.log" \
    "run:(date +%s; DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends code > /tmp/apt.log 2>&1; echo apt-exit-\$?; date +%s)" \
    'run:grep -E "^(E:|Err|W:)" /tmp/apt.log | head -n 20; tail -n 4 /tmp/apt.log; df -h / | tail -n 1' \
    "run:dpkg -l code | grep '^ii' | cut -c1-110"
  rc=$?
  mv "$IMG" target/survey-vscode.img
  pkill -f 'http.server 8765' 2>/dev/null
  exit $rc
  ;;
drop)
  rm -f "target/survey-$1.img"
  ;;
*)
  sed -n '2,19p' "$0"; exit 2
  ;;
esac
