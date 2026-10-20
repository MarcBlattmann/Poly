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
#   - the users polyspec and polyimpl (system users, home in /srv/poly,
#     locked password: reachable only through sudo by an admin)
#   - /srv/poly (directories, the role wrapper, clones, the bare hub,
#     the loop's configuration; rustup and Claude Code in the role users'
#     homes unless --no-install-tools)
#   - /etc/sudoers.d/poly-pipeline (checked with visudo before it is installed)
#   - /etc/systemd/system/poly-pipeline.service (enabled, started only with --start)
#
# It installs no packages: missing ones are listed with the apt command.
# --dry-run prints every change instead of making it.
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
    -h | --help) sed -n '2,24p' "$0"; exit 0 ;;
    *) echo "unknown option $1" >&2; exit 2 ;;
    esac
done

POLY=/srv/poly
HERE=$(cd "$(dirname "$0")" && pwd)
SPEC_USER=polyspec
IMPL_USER=polyimpl
SUDOERS=/etc/sudoers.d/poly-pipeline
UNIT=/etc/systemd/system/poly-pipeline.service

say() { printf '==> %s\n' "$*"; }
run() {
    if [ "$DRY" = 1 ]; then
        printf '    would run:'
        printf ' %q' "$@"
        printf '\n'
    else
        "$@"
    fi
}
as() {
    local u=$1
    shift
    run sudo -u "$u" -H "$@"
}
die() {
    echo "setup: $*" >&2
    exit 1
}

[ "$(id -u)" = 0 ] || [ "$DRY" = 1 ] || die "run as root (sudo $0)"
getent passwd "$RUNNER_USER" >/dev/null || die "no user $RUNNER_USER (the loop runs as it; --runner-user)"
RUNNER_HOME=$(getent passwd "$RUNNER_USER" | cut -d: -f6)
RUNNER_GROUP=$(id -gn "$RUNNER_USER")

say "checking packages"
missing=()
for cmd_pkg in git:git gcc:gcc qemu-system-x86_64:qemu-system-x86 curl:curl visudo:sudo systemctl:systemd realpath:coreutils; do
    command -v "${cmd_pkg%%:*}" >/dev/null || missing+=("${cmd_pkg#*:}")
done
if [ ${#missing[@]} -gt 0 ]; then
    die "missing packages; install them first: sudo apt install ${missing[*]}"
fi
CARGO=$(sudo -u "$RUNNER_USER" -H bash -lc 'command -v cargo' || true)
[ -n "$CARGO" ] || CARGO=$RUNNER_HOME/.cargo/bin/cargo
[ -x "$CARGO" ] || echo "    warning: no cargo for $RUNNER_USER at $CARGO (install rustup as $RUNNER_USER)"
sudo -u "$RUNNER_USER" -H bash -lc 'gh auth status' >/dev/null 2>&1 ||
    echo "    warning: gh is not logged in as $RUNNER_USER (gh auth login): the loop cannot open PRs"

if [ -z "$REPO_URL" ]; then
    REPO_URL=$(git -C "$POLY/runner/repo" remote get-url origin 2>/dev/null ||
        git -C "$HERE" remote get-url origin 2>/dev/null || echo https://github.com/MarcBlattmann/Poly.git)
fi

say "users $SPEC_USER and $IMPL_USER"
for u in "$SPEC_USER:$POLY/spec:spec agent" "$IMPL_USER:$POLY/impl:implementer agent"; do
    IFS=: read -r name home what <<<"$u"
    if getent passwd "$name" >/dev/null; then
        echo "    $name exists"
    else
        run useradd --system --user-group --home-dir "$home" --no-create-home --shell /bin/bash \
            --comment "Poly pipeline $what" "$name"
    fi
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
    "$POLY/spec $SPEC_USER $RUNNER_GROUP 2750"
    "$POLY/impl $IMPL_USER $RUNNER_GROUP 2750"
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
    as "$RUNNER_USER" git init -q --bare "$POLY/git/poly.git"
fi
# Nothing is ever forced into the hub, by anyone.
as "$RUNNER_USER" git -C "$POLY/git/poly.git" config receive.denyNonFastForwards true
git -C "$POLY/runner/repo" remote get-url hub >/dev/null 2>&1 ||
    as "$RUNNER_USER" git -C "$POLY/runner/repo" remote add hub "$POLY/git/poly.git"
as "$RUNNER_USER" git -C "$POLY/runner/repo" fetch -q origin
as "$RUNNER_USER" git -C "$POLY/runner/repo" push -q hub "refs/remotes/origin/main:refs/heads/main"
run chmod -R a+rX "$POLY/git/poly.git"

say "the role checkouts"
for u in "$SPEC_USER:$POLY/spec:Poly spec agent" "$IMPL_USER:$POLY/impl:Poly implementer agent"; do
    IFS=: read -r name home label <<<"$u"
    as "$name" git config --global safe.directory "$POLY/git/poly.git"
    as "$name" git config --global user.name "$label"
    as "$name" git config --global user.email "$name@$(hostname)"
    if [ ! -d "$home/repo/.git" ]; then
        as "$name" git clone -q -o hub "$POLY/git/poly.git" "$home/repo"
    fi
    # The role checkout has no GitHub remote and no credentials.
    if git -C "$home/repo" remote 2>/dev/null | grep -qvx hub; then
        echo "    warning: $home/repo has remotes other than hub: $(git -C "$home/repo" remote | tr '\n' ' ')"
    fi
done

if [ "$INSTALL_TOOLS" = 1 ]; then
    say "rustup and Claude Code for the role users (in their homes under $POLY)"
    for name in "$SPEC_USER" "$IMPL_USER"; do
        home=$(getent passwd "$name" | cut -d: -f6 || echo "$POLY/${name#poly}")
        if [ ! -x "$home/.cargo/bin/rustup" ]; then
            as "$name" bash -c 'curl --proto =https --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y -q --no-modify-path --profile minimal --default-toolchain none'
        fi
        # The toolchain the repository pins (rust-toolchain.toml).
        as "$name" bash -c "cd $home/repo && \$HOME/.cargo/bin/cargo --version"
        if [ ! -x "$home/.local/bin/claude" ]; then
            as "$name" bash -c 'curl -fsSL https://claude.ai/install.sh | bash'
        fi
    done
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
cat >"$tmp" <<EOF
# Written by tools/pipeline/deploy/setup.sh. The pipeline loop ($RUNNER_USER)
# may run the role wrapper, and nothing else, as the two role users. sudo
# resets the environment; the wrapper checks every argument.
$RUNNER_USER ALL=($SPEC_USER) NOPASSWD: $POLY/bin/poly-role spec *
$RUNNER_USER ALL=($IMPL_USER) NOPASSWD: $POLY/bin/poly-role implementer *
EOF
visudo -cqf "$tmp" || die "the generated sudoers rule does not validate"
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
if ! cmp -s "$tmp" "$UNIT" 2>/dev/null; then
    run install -o root -g root -m 0644 "$tmp" "$UNIT"
    run systemctl daemon-reload
fi
run systemctl enable -q poly-pipeline.service
if [ "$START" = 1 ]; then
    run systemctl restart poly-pipeline.service
fi

say "checks"
if [ "$DRY" = 0 ]; then
    if sudo -u "$IMPL_USER" test -r "$POLY/heldout" || sudo -u "$IMPL_USER" test -r "$POLY/generated" ||
        sudo -u "$IMPL_USER" test -r "$POLY/runner" || sudo -u "$IMPL_USER" test -r "$POLY/spec"; then
        die "$IMPL_USER can read the held-out store, the generated tests, the runner or the spec agent's home"
    fi
    echo "    $IMPL_USER cannot read $POLY/heldout, generated, runner or spec: good"
    sudo -u "$RUNNER_USER" sudo -n -l -u "$IMPL_USER" "$POLY/bin/poly-role" implementer sync x y >/dev/null ||
        die "$RUNNER_USER cannot run the implementer wrapper through sudo"
    echo "    $RUNNER_USER may run the role wrapper as $SPEC_USER and $IMPL_USER: good"
fi

cat <<EOF

Done. One-time steps left for you (docs/AGENT_PIPELINE.md, "Setting up the home server"):

 1. Log both role users in to Claude Code with your subscription, and trust their checkouts:
      sudo -iu $SPEC_USER       then:  cd repo && claude   (/login, "Claude account with subscription",
                                       accept "trust this folder", /exit), then: exit
      sudo -iu $IMPL_USER       the same
 2. Review $POLY/runner/pipeline.toml (agent ids, max_parallel, auto_merge, notify.command).
 3. On GitHub (repository settings): allow merge commits and auto-merge; protect main with
    the CI check required. Pipeline PRs must be merged with a merge commit, never squashed.
 4. Try it:  sudo -u $RUNNER_USER -H bash -lc 'cd $POLY/runner/repo && cargo xtask pipeline loop --dry-run'
             sudo -u $RUNNER_USER -H bash -lc 'cd $POLY/runner/repo && cargo xtask pipeline loop --once'
 5. Start:   sudo systemctl start poly-pipeline;  journalctl -u poly-pipeline -f
    Status:  sudo -u $RUNNER_USER -H bash -lc 'cd $POLY/runner/repo && cargo xtask pipeline loop status'
EOF
