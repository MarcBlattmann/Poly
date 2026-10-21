#!/bin/bash
# Set up the Poly agent pipeline on a home server (docs/AGENT_PIPELINE.md,
# ADR 0056). Run as root from a checkout of the repository:
#
#   sudo tools/pipeline/deploy/setup.sh [--dry-run] [--runner-user server]
#        [--repo-url URL] [--no-install-tools] [--start]
#
# Idempotent: run it again after an update; it only creates what is
# missing and resets owners and modes. It changes nothing but:
#
#   - the users polyspec and polyimpl (system users, homes /srv/poly/spec
#     and /srv/poly/impl, locked password: reachable only through sudo)
#   - /srv/poly (directories, the role wrapper, clones, the bare hub,
#     the loop's configuration; rustup, the pinned Rust toolchain and
#     Claude Code in the role users' homes unless --no-install-tools)
#   - the pinned Rust toolchain in the runner user's rustup (not without
#     --no-install-tools)
#   - /etc/sudoers.d/poly-pipeline (checked with visudo before it is installed):
#     the role wrapper as the role users, and `systemctl start|stop|restart|status
#     poly-pipeline` as root, for the runner user, without a password
#   - /etc/systemd/system/poly-pipeline.service (enabled, started only with --start)
#   - /srv/poly/bin/deployed.sha256, the fingerprint of the deploy files it
#     installed: the loop tells the owner when the checkout has newer ones
#
# Before it changes anything it checks the packages and users it needs; a
# user from an earlier attempt (another home, another primary group) stops
# it with the commands that fix it. It never moves, deletes or re-owns an
# existing user's files itself. It installs no packages: missing ones are
# listed with the apt command.
#
# --dry-run prints every change instead of making it. It works without
# root too; then what only root or another user can see (their homes, the
# sudoers file) is not looked at, and the plan shows it as missing.
set -euo pipefail

RUNNER_USER=server
REPO_URL=
INSTALL_TOOLS=1
START=0
DRY=0
while [ $# -gt 0 ]; do
    case "$1" in
    --runner-user) RUNNER_USER=$2; shift 2 ;;
    --repo-url) REPO_URL=$2; shift 2 ;;
    --no-install-tools) INSTALL_TOOLS=0; shift ;;
    --start) START=1; shift ;;
    --dry-run) DRY=1; shift ;;
    -h | --help) sed -n '2,29p' "$0"; exit 0 ;;
    *) echo "unknown option $1" >&2; exit 2 ;;
    esac
done

POLY=/srv/poly
HERE=$(cd "$(dirname "$0")" && pwd)
# Commands run as the role users from a directory they can read.
cd /
SPEC_USER=polyspec
IMPL_USER=polyimpl
# Each role's home, defined once, here. Everything below, the role wrapper
# (poly-role checks the passwd entry against the same paths), the sudoers
# rule, pipeline.toml ([spec] repo, [implementer] repo) and the Claude
# Code logins use these; a role user whose home is anything else is an
# error, never a second place to look.
SPEC_HOME=$POLY/spec
IMPL_HOME=$POLY/impl
# user:home:what:role (the role is poly-role's first argument)
ROLES=("$SPEC_USER:$SPEC_HOME:spec agent:spec" "$IMPL_USER:$IMPL_HOME:implementer agent:implementer")
SUDOERS=/etc/sudoers.d/poly-pipeline
UNIT_NAME=poly-pipeline
UNIT=/etc/systemd/system/$UNIT_NAME.service
SYSTEMCTL=/usr/bin/systemctl
# What setup.sh installed last (update.rs in tools/pipeline/src/scheduler reads it).
STAMP=$POLY/bin/deployed.sha256
# The loop's exit status when it stops to run its new code (update::RESTART_EXIT).
RESTART_EXIT=10
# What setup.sh makes in $POLY; anything else there is from somewhere else.
KNOWN_ENTRIES=" bin git runner inbox spec-inbox spec impl heldout generated lost+found "

say() { printf '==> %s\n' "$*"; }
warn() { printf '    warning: %s\n' "$*"; }
# Every command that changes something goes through run: --dry-run prints it.
run() {
    if [ "$DRY" = 1 ]; then
        printf '    would run:'
        printf ' %q' "$@"
        printf '\n'
    else
        "$@"
    fi
}
# Run as another user (git refuses repositories of other users, so every
# git command runs as the owner of the repository it touches).
as() {
    local u=$1
    shift
    run sudo -u "$u" -H "$@"
}
# A read-only look as another user, also in a dry run. Only root can look
# without a password prompt; without root it fails (unless we are that
# user), and the caller takes the answer as "no".
peek() {
    local u=$1
    shift
    if [ "$ROOT" = 1 ]; then
        sudo -n -u "$u" -H "$@"
    elif [ "$(id -un)" = "$u" ]; then
        "$@"
    else
        return 1
    fi
}
die() {
    echo "setup: $*" >&2
    exit 1
}
problems=()
problem() { problems+=("$*"); }

# The fingerprint of the deploy files in $1 (all but the example
# configuration, which setup.sh never overwrites). Must match deploy_hash
# in tools/pipeline/src/scheduler/update.rs (a test runs this function).
deploy_hash() {
    (cd "$1" && find . -type f ! -name pipeline.example.toml -printf '%P\0' | LC_ALL=C sort -z |
        xargs -0 -r sha256sum | sha256sum | cut -d' ' -f1)
}

# The sudoers rule. A test in tools/pipeline/src/scheduler/tests.rs runs
# this function and compares the text with a snapshot. The service control
# commands match exactly: sudo allows no other arguments, no other unit.
# SYSTEMCTL_STATUS=0 leaves `status` out (systemd before 247 starts its
# pager as root without secure mode under sudo, CVE-2023-26604).
sudoers_text() {
    local status=
    [ "$SYSTEMCTL_STATUS" = 0 ] || status=", \\
    $SYSTEMCTL status $UNIT_NAME"
    cat <<EOF
# Written by tools/pipeline/deploy/setup.sh. The pipeline loop ($RUNNER_USER)
# may run the role wrapper, and nothing else, as the two role users. sudo
# resets the environment; the wrapper checks every argument.
$RUNNER_USER ALL=($SPEC_USER) NOPASSWD: $POLY/bin/poly-role spec *
$RUNNER_USER ALL=($IMPL_USER) NOPASSWD: $POLY/bin/poly-role implementer *
# Service control as root, these exact commands only (docs/AGENT_PIPELINE.md,
# "Updates and service control").
Cmnd_Alias POLY_PIPELINE_SERVICE = $SYSTEMCTL start $UNIT_NAME, \\
    $SYSTEMCTL stop $UNIT_NAME, \\
    $SYSTEMCTL restart $UNIT_NAME$status
$RUNNER_USER ALL=(root) NOPASSWD: POLY_PIPELINE_SERVICE
EOF
}

ROOT=0
[ "$(id -u)" = 0 ] && ROOT=1
if [ "$ROOT" = 0 ]; then
    [ "$DRY" = 1 ] || die "run as root (sudo $0), or with --dry-run to see what it would do"
    say "dry run as $(id -un), not root: other users' homes and the sudoers file are not looked at"
fi
getent passwd "$RUNNER_USER" >/dev/null || die "no user $RUNNER_USER (the loop runs as it; --runner-user)"
case "$RUNNER_USER" in
"$SPEC_USER" | "$IMPL_USER" | root) die "the loop cannot run as $RUNNER_USER (--runner-user)" ;;
esac
RUNNER_HOME=$(getent passwd "$RUNNER_USER" | cut -d: -f6)
RUNNER_GROUP=$(id -gn "$RUNNER_USER")

say "checking packages"
# What `cargo xtask ci` and the loop need on top of an Ubuntu 24.04 install:
# QEMU (the boot tests), a static BusyBox (the BusyBox and root-disk boot
# tests; the CI workflow installs busybox-static too), e2fsprogs (mke2fs and
# e2fsck for the ext2/ext4 disk images), gcc with the static C library (the
# test programs are linked -static), python3 (a root-disk step).
missing=()
need() { [[ " ${missing[*]} " == *" $1 "* ]] || missing+=("$1"); }
for cmd_pkg in git:git gcc:gcc qemu-system-x86_64:qemu-system-x86 mke2fs:e2fsprogs e2fsck:e2fsprogs \
    curl:curl python3:python3 visudo:sudo systemctl:systemd realpath:coreutils; do
    command -v "${cmd_pkg%%:*}" >/dev/null || need "${cmd_pkg#*:}"
done
[ -e /usr/lib/x86_64-linux-gnu/libc.a ] || need libc6-dev
[ -x "$SYSTEMCTL" ] || need systemd
# /bin/busybox from the `busybox` package is dynamically linked: no use.
if ! dpkg-query -W -f='${db:Status-Status}' busybox-static 2>/dev/null | grep -qx installed; then
    need busybox-static
fi
if [ ${#missing[@]} -gt 0 ]; then
    die "missing packages; install them first: sudo apt install ${missing[*]}"
fi
CARGO=$(peek "$RUNNER_USER" bash -lc 'command -v cargo' || true)
[ -n "$CARGO" ] || CARGO=$RUNNER_HOME/.cargo/bin/cargo
if [ ! -x "$CARGO" ]; then
    if [ "$ROOT" = 1 ]; then
        problem "no cargo for $RUNNER_USER (looked on its login PATH and at $CARGO): the loop runs it.
    Install rustup as $RUNNER_USER first (https://rustup.rs), then run setup.sh again:
        sudo -iu $RUNNER_USER bash -c 'curl --proto =https --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain none'"
    else
        warn "cannot see $RUNNER_USER's cargo without root; assuming $CARGO"
    fi
elif [ ! -x "$(dirname "$CARGO")/rustup" ]; then
    problem "$CARGO (the cargo of $RUNNER_USER) is not rustup's: the repository pins its toolchain in
    rust-toolchain.toml, which only rustup reads. Install rustup as $RUNNER_USER (https://rustup.rs) and run setup.sh again."
fi
peek "$RUNNER_USER" bash -lc 'gh auth status' >/dev/null 2>&1 ||
    warn "gh is not installed or not logged in as $RUNNER_USER (sudo -iu $RUNNER_USER gh auth login): the loop cannot open PRs"

if [ -z "$REPO_URL" ]; then
    REPO_URL=$(peek "$RUNNER_USER" git -C "$POLY/runner/repo" remote get-url origin 2>/dev/null ||
        git -c safe.directory='*' -C "$HERE" remote get-url origin 2>/dev/null ||
        echo https://github.com/MarcBlattmann/Poly.git)
fi

say "checking users $SPEC_USER and $IMPL_USER"
# A user from an earlier attempt is never changed here: the fixes are
# printed for the operator, and none of them deletes anything but a clone
# setup.sh makes again. (usermod -d -m refuses to move a home onto an
# existing directory, and rewrites the passwd entry all the same, so it is
# only offered when there is none.)
# The first file or directory at most two levels below $1 that $2 does not own.
foreign() { find "$1" -mindepth 1 -maxdepth 2 ! -user "$2" -printf '%p (owner %u)' -quit 2>/dev/null; }
for r in "${ROLES[@]}"; do
    IFS=: read -r name home _ _ <<<"$r"
    if ! ent=$(getent passwd "$name"); then
        echo "    $name: missing, will be created with home $home"
        if getent group "$name" >/dev/null; then
            problem "there is a group $name but no user $name (left from an earlier user?); useradd --user-group cannot reuse it.
    Remove the group, then run setup.sh again:
        sudo groupdel $name"
        fi
        if [ "$ROOT" = 1 ] && [ -d "$home" ] && [ -n "$(foreign "$home" "$name")" ]; then
            problem "$home holds files of another user, e.g. $(foreign "$home" "$name"), but there is no user $name yet;
    git would refuse that checkout. Look at it (sudo ls -la $home); what an earlier run of setup.sh left
    there (repo/, a clone of the hub) is made again, so remove it, then run setup.sh again:
        sudo rm -rf $home"
        fi
        continue
    fi
    cur=$(cut -d: -f6 <<<"$ent")
    group=$(id -gn "$name")
    ok=1
    if [ "$cur" != "$home" ]; then
        ok=0
        if [ -e "$home" ]; then
            fix="sudo usermod -d $home $name    # moves nothing: $cur stays as it is"
            later="rustup and Claude Code are installed into $home again, and you log in again; delete $cur
    when you no longer need it: sudo rm -rf $cur"
        else
            fix="sudo mkdir -p $POLY && sudo usermod -d $home -m $name    # moves $cur to $home"
            later="its files move along; you still log in to Claude Code as setup.sh says at the end"
        fi
        problem "user $name exists, but its home is $cur, not $home: the role wrapper runs Claude Code, rustup and git
    with HOME=$home, so a login or toolchain in $cur is never used. Fix the passwd entry, then run setup.sh again:
        $fix
    ($later. If usermod says the user is in use, stop its processes first: sudo pkill -u $name)"
    fi
    if [ "$group" != "$name" ]; then
        ok=0
        if getent group "$name" >/dev/null; then
            fix="sudo usermod -g $name $name"
        else
            fix="sudo groupadd --system $name && sudo usermod -g $name $name"
        fi
        problem "the primary group of $name is $group, not its own group $name (setup.sh gives each role user
    its own group; the inbox is readable by group $IMPL_USER only). Give it its own, then run setup.sh again:
        $fix"
    fi
    # Everything in the role's home must be the role user's: git refuses a
    # checkout another user owns.
    if [ "$ROOT" = 1 ] && [ -d "$home" ] && [ -n "$(foreign "$home" "$name")" ]; then
        ok=0
        problem "$home holds files that are not $name's, e.g. $(foreign "$home" "$name"); git refuses such a checkout.
    Give them to $name, then run setup.sh again:
        sudo chown -R $name $home"
    elif [ "$ROOT" = 1 ] && [ -e "$home/repo" ] && ! peek "$name" git -C "$home/repo" rev-parse --git-dir >/dev/null 2>&1; then
        ok=0
        problem "$home/repo is not a git checkout (left from an attempt that failed?). It is only a clone of the hub:
    remove it, and setup.sh clones it again:
        sudo rm -rf $home/repo"
    fi
    extra=$(id -nG "$name" | tr ' ' '\n' | grep -vx "$group" | tr '\n' ' ' || true)
    extra=${extra% }
    if [ -n "$extra" ] && [ "$name" = "$IMPL_USER" ]; then
        # The implementer must not be able to read anything but its own.
        ok=0
        fix=
        for g in $extra; do fix+="
        sudo gpasswd -d $name $g"; done
        problem "$name is also in the group(s) $extra: it could read whatever they can.
    The implementer gets no group but its own; take it out of them, then run setup.sh again:$fix"
    elif [ -n "$extra" ]; then
        warn "$name is also in the group(s) $extra (not needed; from an earlier attempt?): sudo gpasswd -d $name <group>"
    fi
    [ "$ok" = 0 ] || echo "    $name: exists, home $home"
done

say "checking $POLY"
for d in "$POLY/runner/repo" "$POLY/git/poly.git"; do
    if [ -e "$d" ] && [ "$(stat -c %U "$d")" != "$RUNNER_USER" ]; then
        problem "$d belongs to $(stat -c %U "$d"), not $RUNNER_USER, so git refuses it. Remove it (setup.sh makes it again)
    or give it to $RUNNER_USER:
        sudo chown -R $RUNNER_USER:$RUNNER_GROUP $d"
    fi
done
if [ -d "$POLY" ]; then
    for e in "$POLY"/* "$POLY"/.[!.]*; do
        [ -e "$e" ] || continue
        [[ $KNOWN_ENTRIES == *" ${e##*/} "* ]] ||
            warn "$e is not made by setup.sh and nothing uses it (from an earlier attempt?); remove it when you are sure: sudo rm -rf $e"
    done
fi

if [ ${#problems[@]} -gt 0 ]; then
    {
        echo
        for p in "${problems[@]}"; do printf 'setup: %s\n\n' "$p"; done
        echo "setup: stopped before changing anything; fix the above and run setup.sh again."
    } >&2
    exit 1
fi

say "users $SPEC_USER and $IMPL_USER"
for r in "${ROLES[@]}"; do
    IFS=: read -r name home what _ <<<"$r"
    getent passwd "$name" >/dev/null ||
        run useradd --system --user-group --home-dir "$home" --no-create-home --shell /bin/bash \
            --comment "Poly pipeline $what" "$name"
done
# The implementer must not be able to read anything of the runner's group.
if id -nG "$IMPL_USER" 2>/dev/null | tr ' ' '\n' | grep -qx "$RUNNER_GROUP"; then
    die "$IMPL_USER is in group $RUNNER_GROUP: it could read the held-out store; remove it (gpasswd -d $IMPL_USER $RUNNER_GROUP)"
fi

say "directories under $POLY"
# path owner group mode
dirs=(
    "$POLY root root 0755"
    "$POLY/bin root root 0755"
    "$POLY/git $RUNNER_USER $RUNNER_GROUP 0755"
    "$POLY/runner $RUNNER_USER $RUNNER_GROUP 0750"
    "$POLY/runner/work $RUNNER_USER $RUNNER_GROUP 0750"
    "$POLY/runner/state $RUNNER_USER $RUNNER_GROUP 0750"
    "$POLY/runner/log $RUNNER_USER $RUNNER_GROUP 0750"
    "$POLY/inbox $RUNNER_USER $IMPL_USER 2750"
    "$POLY/spec-inbox $RUNNER_USER $SPEC_USER 2750"
    "$SPEC_HOME $SPEC_USER $RUNNER_GROUP 2750"
    "$IMPL_HOME $IMPL_USER $RUNNER_GROUP 2750"
    "$POLY/heldout $SPEC_USER $RUNNER_GROUP 2750"
    "$POLY/generated $SPEC_USER $RUNNER_GROUP 2750"
)
for d in "${dirs[@]}"; do
    read -r path owner group mode <<<"$d"
    run install -d -o "$owner" -g "$group" -m "$mode" "$path"
done
run install -o root -g root -m 0755 "$HERE/poly-role" "$POLY/bin/poly-role"

say "the runner's checkout and the hub"
if [ ! -d "$POLY/runner/repo/.git" ]; then
    as "$RUNNER_USER" git clone -q "$REPO_URL" "$POLY/runner/repo"
fi
if [ ! -d "$POLY/git/poly.git" ]; then
    as "$RUNNER_USER" git init -q --bare --shared=all "$POLY/git/poly.git"
fi
# The role users fetch from the hub but are not in its group, so every file
# git writes there must stay world-readable. The loop pushes under the unit's
# UMask=0027, which would hide new refs and objects from them; git applies
# core.sharedRepository to what it writes instead (the chmod below covers
# what is already there, e.g. a hub made before this setting).
as "$RUNNER_USER" git -C "$POLY/git/poly.git" config core.sharedRepository all
# The hub has main only (git init's default branch may be master): clones
# of it check main out.
as "$RUNNER_USER" git -C "$POLY/git/poly.git" symbolic-ref HEAD refs/heads/main
# Nothing is ever forced into the hub, by anyone.
as "$RUNNER_USER" git -C "$POLY/git/poly.git" config receive.denyNonFastForwards true
peek "$RUNNER_USER" git -C "$POLY/runner/repo" remote get-url hub >/dev/null 2>&1 ||
    as "$RUNNER_USER" git -C "$POLY/runner/repo" remote add hub "$POLY/git/poly.git"
as "$RUNNER_USER" git -C "$POLY/runner/repo" fetch -q origin
as "$RUNNER_USER" git -C "$POLY/runner/repo" push -q hub "refs/remotes/origin/main:refs/heads/main"
run chmod -R a+rX "$POLY/git/poly.git"

say "the role checkouts"
for r in "${ROLES[@]}"; do
    IFS=: read -r name home what _ <<<"$r"
    as "$name" git config --global safe.directory "$POLY/git/poly.git"
    as "$name" git config --global user.name "Poly $what"
    as "$name" git config --global user.email "$name@$(hostname)"
    if [ ! -d "$home/repo/.git" ]; then
        as "$name" git clone -q -o hub "$POLY/git/poly.git" "$home/repo"
    fi
    # The role checkout has no GitHub remote and no credentials.
    remotes=$(peek "$name" git -C "$home/repo" remote 2>/dev/null | tr '\n' ' ' || true)
    if [ -n "${remotes// /}" ] && [ "$remotes" != "hub " ]; then
        warn "$home/repo has remotes other than hub: $remotes"
    fi
    # The loop (the runner user) fetches the role's commits from this
    # checkout, which another user owns. `-c safe.directory` on the fetch is
    # not enough: the upload-pack that git starts in the checkout checks
    # ownership again and refuses ("dubious ownership"). So the runner names
    # both paths safe in its own configuration, once.
    for p in "$home/repo" "$home/repo/.git"; do
        peek "$RUNNER_USER" git config --global --get-all safe.directory 2>/dev/null | grep -qxF "$p" ||
            as "$RUNNER_USER" git config --global --add safe.directory "$p"
    done
done

if [ "$INSTALL_TOOLS" = 1 ]; then
    say "rustup, the pinned Rust toolchain and Claude Code for the role users (in $SPEC_HOME and $IMPL_HOME)"
    for r in "${ROLES[@]}"; do
        IFS=: read -r name home _ _ <<<"$r"
        if [ ! -x "$home/.cargo/bin/rustup" ]; then
            as "$name" bash -c 'curl --proto =https --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y -q --no-modify-path --profile minimal --default-toolchain none'
        fi
        # The toolchain rust-toolchain.toml pins, with its components and
        # targets, installed explicitly (rustup's automatic install on first
        # use is deprecated), then checked with that automatic install off.
        # shellcheck disable=SC2016 # "$1" is expanded by the role user's shell
        as "$name" bash -c 'cd "$1" && ~/.cargo/bin/rustup toolchain install && RUSTUP_AUTO_INSTALL=0 ~/.cargo/bin/cargo --version' \
            _ "$home/repo"
        if [ ! -x "$home/.local/bin/claude" ]; then
            as "$name" bash -c 'curl -fsSL https://claude.ai/install.sh | bash'
        fi
    done
    say "the pinned Rust toolchain for $RUNNER_USER"
    # shellcheck disable=SC2016 # expanded by the runner's shell
    as "$RUNNER_USER" bash -c 'cd "$1" && "$2/rustup" toolchain install && RUSTUP_AUTO_INSTALL=0 "$2/cargo" --version' \
        _ "$POLY/runner/repo" "$(dirname "$CARGO")"
fi

say "configuration $POLY/runner/pipeline.toml"
if [ ! -f "$POLY/runner/pipeline.toml" ]; then
    run install -o "$RUNNER_USER" -g "$RUNNER_GROUP" -m 0640 "$HERE/pipeline.example.toml" "$POLY/runner/pipeline.toml"
else
    echo "    kept (compare with $HERE/pipeline.example.toml after updates)"
fi

say "sudoers rule $SUDOERS"
tmp=$(mktemp)
trap 'rm -f "$tmp"' EXIT
SYSTEMD_VERSION=$("$SYSTEMCTL" --version | awk 'NR == 1 { print $2 + 0 }')
SYSTEMCTL_STATUS=1
if [ "${SYSTEMD_VERSION:-0}" -lt 247 ]; then
    SYSTEMCTL_STATUS=0
    warn "systemd $SYSTEMD_VERSION is older than 247: no \`sudo systemctl status $UNIT_NAME\` (its pager would run as root)"
fi
sudoers_text >"$tmp"
visudo -cqf "$tmp" || die "the generated sudoers rule does not validate"
[ "$DRY" = 0 ] || sed "s/^/      | /" "$tmp"
if ! cmp -s "$tmp" "$SUDOERS" 2>/dev/null; then
    run install -o root -g root -m 0440 "$tmp" "$SUDOERS"
fi
[ "$DRY" = 1 ] || visudo -cq || die "sudoers no longer validates; remove $SUDOERS"

say "systemd unit $UNIT"
cat >"$tmp" <<EOF
# Written by tools/pipeline/deploy/setup.sh (docs/AGENT_PIPELINE.md, ADR 0056).
[Unit]
Description=Poly agent pipeline loop
Documentation=file://$POLY/runner/repo/docs/AGENT_PIPELINE.md
Wants=network-online.target
After=network-online.target

[Service]
Type=simple
User=$RUNNER_USER
Group=$RUNNER_GROUP
WorkingDirectory=$POLY/runner/repo
Environment=POLY_PIPELINE_CONFIG=$POLY/runner/pipeline.toml
Environment=PATH=$(dirname "$CARGO"):$RUNNER_HOME/.local/bin:/usr/local/bin:/usr/bin:/bin
Environment=CARGO_BUILD_JOBS=3
ExecStart=$CARGO xtask pipeline loop
# The loop exits $RESTART_EXIT when main changed its own code and the new code
# builds: not a failure; Restart= starts it again, and cargo runs the new code.
SuccessExitStatus=$RESTART_EXIT
Restart=always
RestartSec=60
TimeoutStopSec=60
UMask=0027
# A background job on a 4-core, 15 GB machine: lower priority, 3 cores,
# memory capped below what the desktop and the system need.
Nice=10
IOSchedulingClass=best-effort
IOSchedulingPriority=7
CPUWeight=50
CPUQuota=300%
MemoryHigh=11G
MemoryMax=12G
TasksMax=4096
# No NoNewPrivileges=: the loop starts the role wrapper through sudo.

[Install]
WantedBy=multi-user.target
EOF
[ "$DRY" = 0 ] || sed "s/^/      | /" "$tmp"
if ! cmp -s "$tmp" "$UNIT" 2>/dev/null; then
    run install -o root -g root -m 0644 "$tmp" "$UNIT"
    run systemctl daemon-reload
    if [ "$START" = 0 ] && systemctl is-active -q "$UNIT_NAME" 2>/dev/null; then
        warn "the running loop keeps the old unit until it starts again (its next self-update, or
    sudo systemctl restart $UNIT_NAME, which stops the steps in progress; they resume from their state)"
    fi
fi
run systemctl enable -q poly-pipeline.service
if [ "$START" = 1 ]; then
    run systemctl restart poly-pipeline.service
fi

say "checks"
if [ "$DRY" = 0 ]; then
    for r in "${ROLES[@]}"; do
        IFS=: read -r name home _ _ <<<"$r"
        [ "$(getent passwd "$name" | cut -d: -f6)" = "$home" ] || die "$name's home is not $home"
    done
    if sudo -u "$IMPL_USER" test -r "$POLY/heldout" || sudo -u "$IMPL_USER" test -r "$POLY/generated" ||
        sudo -u "$IMPL_USER" test -r "$POLY/runner" || sudo -u "$IMPL_USER" test -r "$SPEC_HOME" ||
        sudo -u "$IMPL_USER" test -r "$POLY/spec-inbox"; then
        die "$IMPL_USER can read the held-out store, the generated tests, the runner, the spec agent's home or inbox"
    fi
    echo "    $IMPL_USER cannot read $POLY/heldout, generated, runner, spec or spec-inbox: good"
    sudo -u "$RUNNER_USER" sudo -n -l -u "$IMPL_USER" "$POLY/bin/poly-role" implementer sync x y >/dev/null ||
        die "$RUNNER_USER cannot run the implementer wrapper through sudo"
    sudo -u "$RUNNER_USER" sudo -n -l -u "$SPEC_USER" "$POLY/bin/poly-role" spec sync x y >/dev/null ||
        die "$RUNNER_USER cannot run the spec wrapper through sudo"
    echo "    $RUNNER_USER may run the role wrapper as $SPEC_USER and $IMPL_USER: good"
    sudo -u "$RUNNER_USER" sudo -n -l "$SYSTEMCTL" restart "$UNIT_NAME" >/dev/null ||
        die "$RUNNER_USER cannot run systemctl restart $UNIT_NAME through sudo"
    echo "    $RUNNER_USER may start, stop and restart $UNIT_NAME without a password: good"
else
    echo "    (not in a dry run)"
fi

say "deploy fingerprint $STAMP"
# Last: only a complete install is recorded. The loop compares it with the
# deploy files of its checkout and tells the owner when they differ.
deploy_hash "$HERE" >"$tmp"
[ "$DRY" = 0 ] || sed "s/^/      | /" "$tmp"
cmp -s "$tmp" "$STAMP" 2>/dev/null || run install -o root -g root -m 0644 "$tmp" "$STAMP"

if [ "$DRY" = 1 ]; then
    echo
    echo "Dry run: nothing was changed. Without --dry-run, setup.sh makes the changes above and then lists"
    echo "the one-time steps left (the Claude Code logins: sudo -u $SPEC_USER $POLY/bin/poly-role spec login, ...)."
    exit 0
fi
cat <<EOF

Done. One-time steps left for you (docs/AGENT_PIPELINE.md, "Setting up the home server"):

 1. Log both role users in to Claude Code with your subscription, and trust their checkouts.
    \`login\` starts claude exactly as the pipeline will (HOME=$SPEC_HOME or $IMPL_HOME, in repo/):
      sudo -u $SPEC_USER $POLY/bin/poly-role spec login
          /login, "Claude account with subscription", accept "trust this folder", /exit
      sudo -u $IMPL_USER $POLY/bin/poly-role implementer login
          the same
 2. Review $POLY/runner/pipeline.toml (agent ids, max_parallel, auto_merge, notify.command).
 3. On GitHub (repository settings): allow merge commits and auto-merge; protect main with
    the CI check required. Pipeline PRs must be merged with a merge commit, never squashed.
 4. Try it:  sudo -u $RUNNER_USER -H bash -lc 'cd $POLY/runner/repo && cargo xtask pipeline loop --dry-run'
             sudo -u $RUNNER_USER -H bash -lc 'cd $POLY/runner/repo && cargo xtask pipeline loop --once'
 5. Start:   sudo systemctl start poly-pipeline;  journalctl -u poly-pipeline -f
    Status:  sudo -u $RUNNER_USER -H bash -lc 'cd $POLY/runner/repo && cargo xtask pipeline loop status'
    Watch:   sudo -u $RUNNER_USER -H bash -lc 'cd $POLY/runner/repo && cargo xtask pipeline loop watch'   (q quits)

Routine operation needs no commands: the loop restarts itself with new code from main. As $RUNNER_USER:
    cargo xtask pipeline loop pause [--role spec|implementer] [--for 4h] [--reason ...];  loop resume
    sudo systemctl start|stop|restart|status $UNIT_NAME      (no password)
When deploy files change, \`loop status\` and the human queue say so: run this script again.
EOF
