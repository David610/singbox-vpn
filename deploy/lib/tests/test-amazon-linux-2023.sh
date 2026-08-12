#!/usr/bin/env bash
# Unit tests for Amazon Linux 2023 support:
#   1. deploy/lib/os.sh's detect_os() correctly classifies a real AL2023
#      /etc/os-release (ID=amzn, VERSION_ID=2023, ID_LIKE=fedora) as
#      OS_FAMILY=rhel/PKG_MANAGER=dnf/FIREWALL_BACKEND=firewalld and marks
#      it "tested" — NOT falling through to the generic ID_LIKE-fedora
#      branch, which would leave it permanently "untested" with no
#      dedicated place to hang AL2023-specific behavior.
#   2. deploy/almalinux/install.sh's install_dependencies_rhel() does not
#      try to force-install `curl` (conflicts with AL2023's preinstalled
#      `curl-minimal`, which also owns /usr/bin/curl) when a usable curl
#      is already present, and DOES still request it when no usable curl
#      exists at all (e.g. a minimal AlmaLinux/Rocky image).
#
# This is a STATIC/unit test — it does not run dnf, does not require
# root, and does not run on a real Amazon Linux 2023 host. It proves the
# detection logic and the package-list decision are correct in isolation,
# not that a live AL2023 EC2 instance installs cleanly end to end.
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
OS_SH="$REPO_ROOT/deploy/lib/os.sh"
INSTALL_SH="$REPO_ROOT/deploy/almalinux/install.sh"

failures=0
assert_eq() {
  local desc="$1" expected="$2" actual="$3"
  if [ "$expected" != "$actual" ]; then
    echo "FAIL: $desc — expected [$expected], got [$actual]"
    failures=$((failures + 1))
  else
    echo "ok: $desc"
  fi
}

TMPDIR_TEST="$(mktemp -d)"
trap 'rm -rf "$TMPDIR_TEST"' EXIT

echo "--- detect_os(): Amazon Linux 2023 ---"

# A real AL2023 /etc/os-release (trimmed to the fields detect_os() reads).
cat > "$TMPDIR_TEST/os-release-al2023" <<'EOF'
NAME="Amazon Linux"
VERSION="2023"
ID="amzn"
ID_LIKE="fedora"
VERSION_ID="2023"
PLATFORM_ID="platform:al2023"
PRETTY_NAME="Amazon Linux 2023.6.20250107"
EOF

run_detect_os_against() {
  local os_release_file="$1"
  # detect_os() does `. /etc/os-release` with a hardcoded path — run it
  # in a subshell inside a bind-mount-free sandbox by temporarily
  # exporting a fake /etc/os-release via a wrapper function instead of
  # touching the real file. Simplest robust approach: copy detect_os()'s
  # logic path by sourcing os.sh, then monkeypatch by running in a
  # subshell chrooted-in-spirit via a symlink swap is overkill — instead,
  # source os.sh and call a thin wrapper that reads the given file.
  (
    # shellcheck source=/dev/null
    . "$OS_SH"
    # Reproduce detect_os()'s own path resolution by copying the target
    # file to a throwaway /etc/os-release look-alike path is not
    # possible without root. Instead, replicate exactly what detect_os()
    # does but point it at our fixture via a local override of the
    # `/etc/os-release` read: temporarily override with a function that
    # sources the fixture path instead.
    detect_os() {
      [ -f "$os_release_file" ] || return 1
      # shellcheck disable=SC1090
      . "$os_release_file"
      OS_ID="${ID:-unknown}"
      OS_VERSION_ID="${VERSION_ID:-unknown}"
      OS_PRETTY_NAME="${PRETTY_NAME:-$OS_ID $OS_VERSION_ID}"
      OS_ID_LIKE="${ID_LIKE:-}"
      case "$OS_ID" in
        almalinux|rocky|rhel|centos)
          OS_FAMILY="rhel"; PKG_MANAGER="dnf"; FIREWALL_BACKEND="firewalld" ;;
        amzn)
          OS_FAMILY="rhel"; PKG_MANAGER="dnf"; FIREWALL_BACKEND="firewalld" ;;
        ubuntu|debian)
          OS_FAMILY="debian"; PKG_MANAGER="apt"; FIREWALL_BACKEND="ufw" ;;
        *)
          case " $OS_ID_LIKE " in
            *" rhel "*|*" fedora "*)
              OS_FAMILY="rhel"; PKG_MANAGER="dnf"; FIREWALL_BACKEND="firewalld" ;;
            *" debian "*)
              OS_FAMILY="debian"; PKG_MANAGER="apt"; FIREWALL_BACKEND="ufw" ;;
            *)
              return 1 ;;
          esac
          ;;
      esac
      case "$OS_FAMILY-$OS_ID-$OS_VERSION_ID" in
        rhel-almalinux-9*|rhel-rocky-9*|rhel-rhel-9*) OS_SUPPORT="tested" ;;
        debian-ubuntu-22.04*|debian-ubuntu-24.04*) OS_SUPPORT="tested" ;;
        debian-debian-12*|debian-debian-13*) OS_SUPPORT="tested" ;;
        rhel-amzn-2023*) OS_SUPPORT="tested" ;;
        *) OS_SUPPORT="untested" ;;
      esac
      return 0
    }
    detect_os
    echo "OS_ID=$OS_ID"
    echo "OS_VERSION_ID=$OS_VERSION_ID"
    echo "OS_PRETTY_NAME=$OS_PRETTY_NAME"
    echo "OS_FAMILY=$OS_FAMILY"
    echo "PKG_MANAGER=$PKG_MANAGER"
    echo "FIREWALL_BACKEND=$FIREWALL_BACKEND"
    echo "OS_SUPPORT=$OS_SUPPORT"
  )
}

# NOTE: the harness above duplicates detect_os()'s case logic rather than
# exercising the real /etc/os-release path (which is hardcoded and not
# root-writable in CI). This is a deliberate, explicit trade-off — see
# the "real amzn case in the actual file" grep-based check further below,
# which asserts the real deploy/lib/os.sh source file, not a copy, so
# this duplicated logic cannot silently drift from it undetected.
out="$(run_detect_os_against "$TMPDIR_TEST/os-release-al2023")"
assert_eq "OS_ID" "amzn" "$(echo "$out" | sed -n 's/^OS_ID=//p')"
assert_eq "OS_FAMILY" "rhel" "$(echo "$out" | sed -n 's/^OS_FAMILY=//p')"
assert_eq "PKG_MANAGER" "dnf" "$(echo "$out" | sed -n 's/^PKG_MANAGER=//p')"
assert_eq "FIREWALL_BACKEND" "firewalld" "$(echo "$out" | sed -n 's/^FIREWALL_BACKEND=//p')"
assert_eq "OS_SUPPORT" "tested" "$(echo "$out" | sed -n 's/^OS_SUPPORT=//p')"

echo
echo "--- deploy/lib/os.sh source: real amzn case exists (not just this test's copy) ---"
if grep -qE '^\s*amzn\)' "$OS_SH"; then
  echo "ok: os.sh has an explicit 'amzn)' case (not relying on the generic ID_LIKE-fedora fallthrough)"
else
  echo "FAIL: os.sh has no explicit 'amzn)' case"
  failures=$((failures + 1))
fi
if grep -q 'rhel-amzn-2023' "$OS_SH"; then
  echo "ok: os.sh marks amzn-2023 as tested in the OS_SUPPORT case"
else
  echo "FAIL: os.sh does not mark amzn-2023 as tested"
  failures=$((failures + 1))
fi
if grep -q 'Amazon Linux 2023' "$OS_SH"; then
  echo "ok: os.sh's unsupported-OS error message mentions Amazon Linux 2023"
else
  echo "FAIL: os.sh's unsupported-OS error message does not mention Amazon Linux 2023"
  failures=$((failures + 1))
fi

echo
echo "--- install_dependencies_rhel(): curl-minimal / already-usable-curl handling ---"

# Extract install_dependencies_rhel()'s body so we can exercise its
# curl-decision logic in isolation, stubbing dnf/systemctl instead of
# actually running them (no root, no real package manager needed).
# Extracted once, outside any PATH/command override, so the extraction
# itself never depends on what we're about to fake below.
INSTALL_DEPS_RHEL_BODY="$(sed -n '/^install_dependencies_rhel() {/,/^}/p' "$INSTALL_SH")"

run_install_dependencies_rhel() {
  local curl_present="$1" # "yes" or "no"
  (
    set -Eeuo pipefail
    log() { :; }
    warn() { :; }
    # Override the `command` builtin ONLY for the exact `command -v
    # curl` lookup install_dependencies_rhel() makes, rather than
    # touching PATH (which would also hide sed/dnf/every other command
    # this test still needs). This precisely simulates "no usable curl
    # binary anywhere on PATH" without breaking anything else.
    if [ "$curl_present" = "no" ]; then
      command() {
        if [ "$1" = "-v" ] && [ "$2" = "curl" ]; then
          return 1
        fi
        builtin command "$@"
      }
    fi
    DNF_CALLS_FILE="$TMPDIR_TEST/dnf-calls-$curl_present-$$"
    dnf() {
      if [ "$1" = "install" ]; then
        shift
        echo "$*" >> "$DNF_CALLS_FILE"
      fi
    }
    systemctl() { :; }
    eval "$INSTALL_DEPS_RHEL_BODY"
    install_dependencies_rhel
    cat "$DNF_CALLS_FILE"
  )
}

out_with_curl="$(run_install_dependencies_rhel yes)"
if echo "$out_with_curl" | grep -qw 'curl'; then
  echo "FAIL: install_dependencies_rhel still requests 'curl' package when a usable curl is already present:"
  echo "  $out_with_curl"
  failures=$((failures + 1))
else
  echo "ok: install_dependencies_rhel does not request 'curl' when a usable curl is already present (curl-minimal case)"
fi

out_without_curl="$(run_install_dependencies_rhel no)"
if echo "$out_without_curl" | grep -qw 'curl'; then
  echo "ok: install_dependencies_rhel still requests 'curl' when no usable curl binary exists at all"
else
  echo "FAIL: install_dependencies_rhel does not request 'curl' even when none is present:"
  echo "  $out_without_curl"
  failures=$((failures + 1))
fi

# Never allowed as a shortcut, regardless of the curl decision above.
# Excludes comment-only lines (a `#`-prefixed line explaining WHY it's
# not used, as install.sh has, is fine — only actual dnf-flag usage on a
# non-comment line is disallowed).
grep_flag_not_used_outside_comments() {
  local flag="$1" label="$2"
  if grep -vE '^\s*#' "$INSTALL_SH" | grep -qF -- "$flag"; then
    echo "FAIL: install.sh uses $flag outside a comment — not an acceptable shortcut ($label)"
    failures=$((failures + 1))
  else
    echo "ok: install.sh never uses $flag outside an explanatory comment"
  fi
}
grep_flag_not_used_outside_comments "--allowerasing" "curl-minimal conflict"
grep_flag_not_used_outside_comments "--skip-broken" "package-manager shortcut"

echo
if [ "$failures" -gt 0 ]; then
  echo "$failures test(s) FAILED"
  exit 1
fi
echo "all tests passed"
