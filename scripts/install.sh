#!/bin/sh
# silicon omni — one command to have it.
#
#   curl -fsSL https://omni.teamofsilicons.com/install.sh | sh
#
# What this does: works out what machine it is on, downloads the matching
# release from PyPI, checks it against the hash the index published, and puts
# four native binaries — omni, so, silicon-omni, omnid — on your PATH.
#
# It needs no Python and no Rust. The published wheel is a zip with
# already-compiled executables inside it, so this reads it as what it is
# rather than installing a language runtime to unpack one file. If there is no
# release for this machine it falls back to building from source with Cargo,
# and says so before it starts.
#
# Options, as flags or environment variables:
#   --version X      OMNI_VERSION   a release to pin, instead of the latest
#   --bin DIR        OMNI_BIN       where to install (default ~/.omni/bin)
#   --no-path        OMNI_NO_PATH   do not touch any shell startup file
#   --from-source    OMNI_SOURCE    build with Cargo even if a release fits
#   --help
#
# The canonical copy of this script lives in the silicon-omni repository at
# scripts/install.sh; omni.teamofsilicons.com/install.sh redirects to it.

set -eu

PACKAGE="silicon-omni"
DIST="silicon_omni"
INDEX="${OMNI_INDEX:-https://pypi.org/simple/silicon-omni/}"
BINARIES="omni so silicon-omni omnid"

VERSION="${OMNI_VERSION:-}"
BIN="${OMNI_BIN:-}"
NO_PATH="${OMNI_NO_PATH:-}"
SOURCE="${OMNI_SOURCE:-}"

# ------------------------------------------------------------------ talking

if [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then
    BOLD=$(printf '\033[1m'); DIM=$(printf '\033[2m'); OFF=$(printf '\033[0m')
else
    BOLD=""; DIM=""; OFF=""
fi

say() { printf '  %s\n' "$*"; }
step() { printf '  %s%s%s\n' "$DIM" "$*" "$OFF"; }
die() { printf '\n  %somni install: %s%s\n\n' "$BOLD" "$*" "$OFF" >&2; exit 1; }

usage() {
    sed -n '2,26p' "$0" | sed 's/^# \{0,1\}//'
    exit 0
}

while [ $# -gt 0 ]; do
    case "$1" in
        --version) VERSION="${2:-}"; shift 2 || die "--version needs a value" ;;
        --version=*) VERSION="${1#*=}"; shift ;;
        --bin) BIN="${2:-}"; shift 2 || die "--bin needs a directory" ;;
        --bin=*) BIN="${1#*=}"; shift ;;
        --no-path) NO_PATH=1; shift ;;
        --from-source) SOURCE=1; shift ;;
        -h|--help) usage ;;
        *) die "unknown option $1 (try --help)" ;;
    esac
done

[ -n "$BIN" ] || BIN="${OMNI_HOME:-$HOME/.omni}/bin"

# ------------------------------------------------------------------ the tools

have() { command -v "$1" >/dev/null 2>&1; }

# Pin the protocol whenever the address is an https one, which every default
# here is. That is what stops a redirect from walking the download down to
# plain http; a URL that started out local is left alone so the index can be
# pointed somewhere else for a test or a mirror.
secure() { case "$1" in https://*) printf -- "--proto\n=https\n--tlsv1.2\n" ;; esac; }

if have curl; then
    fetch() { curl -fsSL $(secure "$1") "$1"; }
    download() { curl -fsSL $(secure "$1") -o "$2" "$1"; }
elif have wget; then
    fetch() { wget -qO- "$1"; }
    download() { wget -qO "$2" "$1"; }
else
    die "this needs curl or wget to reach the internet"
fi

digest() {
    if have shasum; then shasum -a 256 "$1" | cut -d' ' -f1
    elif have sha256sum; then sha256sum "$1" | cut -d' ' -f1
    else echo ""
    fi
}

# A wheel is a zip. Any of these can open one; a machine has at least one.
unpack() {
    archive="$1"; into="$2"
    if have unzip; then unzip -qo "$archive" -d "$into" && return 0; fi
    if have bsdtar; then bsdtar -xf "$archive" -C "$into" && return 0; fi
    if have python3; then python3 -m zipfile -e "$archive" "$into" && return 0; fi
    # BSD tar, which is what `tar` is on macOS, reads zip. GNU tar does not,
    # and fails here rather than producing half a directory.
    if tar -xf "$archive" -C "$into" 2>/dev/null; then return 0; fi
    return 1
}

# --------------------------------------------------------------- the machine

case "$(uname -s)" in
    Darwin) SYSTEM=macos ;;
    Linux)  SYSTEM=linux ;;
    MINGW*|MSYS*|CYGWIN*)
        die "omni needs Unix-domain sockets; on Windows, run it inside WSL" ;;
    *) die "omni does not have a release for $(uname -s)" ;;
esac

case "$(uname -m)" in
    arm64|aarch64) MACHINE=arm ;;
    x86_64|amd64)  MACHINE=intel ;;
    *) die "omni does not have a release for $(uname -m)" ;;
esac

# A pattern, not a fixed tag: the macOS deployment target moves between
# releases and matching it exactly would break on the one that changed it.
case "$SYSTEM:$MACHINE" in
    macos:arm)   PLATFORM='macosx_[0-9_]*_arm64' ;;
    macos:intel) PLATFORM='macosx_[0-9_]*_x86_64' ;;
    linux:arm)   PLATFORM='manylinux_[0-9_]*_aarch64' ;;
    linux:intel) PLATFORM='manylinux_[0-9_]*_x86_64' ;;
esac

printf '\n  %ssilicon omni%s\n' "$BOLD" "$OFF"
step "$SYSTEM/$MACHINE"

# ------------------------------------------------------------ from a release

# The simple index is one anchor per file, with the hash in the fragment.
# That makes both the URL and the checksum greppable without a JSON parser.
pick_release() {
    listing=$(fetch "$INDEX" 2>/dev/null) || return 1
    [ -n "$listing" ] || return 1

    match=$(printf '%s\n' "$listing" \
        | tr '<' '\n' \
        | grep -o "href=\"[^\"]*${DIST}-[0-9][0-9.]*-py3-none-${PLATFORM}\.whl#sha256=[0-9a-f]*\"" \
        | sed 's/^href="//; s/"$//' \
        | awk -v want="$VERSION" '
            {
              file = $0
              sub(/.*\//, "", file)
              sub(/#.*/, "", file)
              n = split(file, part, "-")
              version = part[2]
              if (want != "") { if (version == want) print version "\t" $0; next }
              # No pre-releases when nobody asked for one: a version with a
              # letter in it is a1/b2/rc1 and is not what `latest` means.
              if (version ~ /[a-zA-Z]/) next
              print version "\t" $0
            }' \
        | sort -t. -k1,1n -k2,2n -k3,3n \
        | tail -n 1)

    [ -n "$match" ] || return 1
    RELEASE=$(printf '%s' "$match" | cut -f1)
    URL=$(printf '%s' "$match" | cut -f2 | sed 's/#.*//')
    WANT=$(printf '%s' "$match" | cut -f2 | sed 's/.*#sha256=//')
    return 0
}

install_release() {
    step "resolving $PACKAGE"
    pick_release || return 1
    say "found $PACKAGE $RELEASE"

    WORK=$(mktemp -d "${TMPDIR:-/tmp}/omni-install.XXXXXX") || die "cannot make a temporary directory"
    # shellcheck disable=SC2064
    trap "rm -rf '$WORK'" EXIT INT TERM

    step "downloading $(basename "$URL")"
    download "$URL" "$WORK/omni.whl" || die "could not download $URL"

    got=$(digest "$WORK/omni.whl")
    if [ -z "$got" ]; then
        say "no sha256 tool here, so the download is unverified"
    elif [ "$got" != "$WANT" ]; then
        die "the download does not match the hash the index published
    expected $WANT
    got      $got"
    else
        step "sha256 ok"
    fi

    unpack "$WORK/omni.whl" "$WORK/out" >/dev/null 2>&1 || {
        mkdir -p "$WORK/out"
        unpack "$WORK/omni.whl" "$WORK/out" >/dev/null 2>&1
    } || die "could not unpack the release; install unzip and try again"

    for name in $BINARIES; do
        [ -f "$WORK/out/omni/bin/$name" ] || die "the release is missing $name"
    done

    mkdir -p "$BIN"
    for name in $BINARIES; do
        cp "$WORK/out/omni/bin/$name" "$BIN/$name.new"
        chmod 755 "$BIN/$name.new"
        # Replace by rename, so a running omnid is never a half-written file.
        mv -f "$BIN/$name.new" "$BIN/$name"
    done
    INSTALLED="$RELEASE"
    return 0
}

# ------------------------------------------------------------- or from source

install_source() {
    have cargo || die "no release fits this machine and Cargo is not installed.
    Install Rust from https://rustup.rs and run this again."
    say "building from source; this takes a few minutes"
    spec=""
    [ -n "$VERSION" ] && spec="@$VERSION"
    cargo install --quiet --root "${BIN%/bin}" \
        "omni-daemon$spec" "silicon-omni-cli$spec" \
        || die "cargo could not build omni"
    INSTALLED="from source"
    return 0
}

if [ -n "$SOURCE" ]; then
    install_source
elif ! install_release; then
    say "no published release fits $SYSTEM/$MACHINE"
    install_source
fi

# ------------------------------------------------------------------ the PATH

on_path() {
    case ":${PATH}:" in *":$BIN:"*) return 0 ;; *) return 1 ;; esac
}

# Written for whichever shell is actually in use. One file, not all of them:
# a duplicated export in three startup files is a thing people then have to
# find and remove by hand.
rc_file() {
    case "${SHELL:-}" in
        */zsh)  printf '%s\n' "${ZDOTDIR:-$HOME}/.zshrc" ;;
        */bash)
            if [ "$SYSTEM" = macos ] && [ -f "$HOME/.bash_profile" ]; then
                printf '%s\n' "$HOME/.bash_profile"
            else
                printf '%s\n' "$HOME/.bashrc"
            fi ;;
        */fish) printf '%s\n' "$HOME/.config/fish/config.fish" ;;
        *)      printf '%s\n' "$HOME/.profile" ;;
    esac
}

add_to_path() {
    rc=$(rc_file)
    if [ -f "$rc" ] && grep -q "omni/bin\|$BIN" "$rc" 2>/dev/null; then
        step "$(basename "$rc") already points at it"
        return 0
    fi
    mkdir -p "$(dirname "$rc")"
    {
        printf '\n# silicon omni\n'
        case "$rc" in
            *fish*) printf 'fish_add_path %s\n' "$BIN" ;;
            *)      printf 'export PATH="%s:$PATH"\n' "$BIN" ;;
        esac
    } >> "$rc"
    say "added $BIN to PATH in $rc"
    NEEDS_RELOAD="$rc"
}

NEEDS_RELOAD=""
if [ -n "$NO_PATH" ]; then
    step "leaving PATH alone, as asked"
elif on_path; then
    step "$BIN is already on PATH"
else
    add_to_path
fi

# ------------------------------------------------------------------- and then

if [ -x "$BIN/omni" ]; then
    got=$("$BIN/omni" --version 2>/dev/null || echo "")
    [ -n "$got" ] || die "installed $BIN/omni but it will not run here"
    say "installed ${got#silicon-omni } — omni, so, silicon-omni, omnid"
else
    say "installed $INSTALLED"
fi

printf '\n'
if [ -n "$NEEDS_RELOAD" ]; then
    printf '  %sOpen a new terminal%s, or: %s. exec $SHELL%s\n' "$BOLD" "$OFF" "$DIM" "$OFF"
    printf '\n'
fi
printf '  %somni chat my-session%s        talk to it here\n' "$BOLD" "$OFF"
printf '  %somni web%s                    serve it to a website on localhost\n' "$BOLD" "$OFF"
printf '  %somni web connect%s            a code to let one site in\n' "$BOLD" "$OFF"
printf '\n'
printf '  %sYou bring the CLIs: claude, codex, or agy. omni offers%s\n' "$DIM" "$OFF"
printf '  %sonly the ones installed and signed in — `omni providers`.%s\n' "$DIM" "$OFF"
printf '\n'
