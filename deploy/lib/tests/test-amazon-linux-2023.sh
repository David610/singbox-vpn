#!/usr/bin/env bash
# Tests for Amazon Linux 2023 support:
#   1. FUNCTIONAL: deploy/lib/os.sh's REAL detect_os() (sourced, not
#      duplicated) correctly classifies a real AL2023 /etc/os-release
#      (ID=amzn, VERSION_ID=2023, ID_LIKE=fedora) as
#      OS_FAMILY=rhel/PKG_MANAGER=dnf/FIREWALL_BACKEND=firewalld and marks
#      it "ci-tested" — NOT falling through to the generic ID_LIKE-fedora
#      branch, which would leave it permanently "untested" with no
#      dedicated place to hang AL2023-specific behavior. detect_os() is
#      exercised via its OS_RELEASE_FILE override (see deploy/lib/os.sh)
#      against a fixture file, so this is the actual production function,
#      not a reimplementation of its logic.
#   2. FUNCTIONAL: the same real detect_os(), against a second fixture
#      with a plain `ID_LIKE="fedora"` and no explicit amzn/almalinux/
#      rocky/rhel/centos ID (a hypothetical future fedora-based distro),
#      still lands on the generic ID_LIKE-fedora fallback path and
#      classifies as OS_SUPPORT=untested. This proves the explicit
#      `amzn)` case and the generic fallback case are two genuinely
#      distinct code paths, not one path with a name that happens to
#      match "amzn" too.
#   3. FUNCTIONAL: deploy/almalinux/install.sh's REAL
#      install_dependencies_rhel() (its body is extracted from the real
#      source file and eval'd with dnf/systemctl stubbed out — not
#      reimplemented) does not try to force-install `curl` (conflicts
#      with AL2023's preinstalled `curl-minimal`, which also owns
#      /usr/bin/curl) when a usable curl is already present, and DOES
#      still request it when no usable curl exists at all (e.g. a
#      minimal AlmaLinux/Rocky image).
#   4. STATIC: a few structural greps against the real os.sh source,
#      clearly labeled as static/structural checks (not a substitute for
#      #1/#2 above, which actually execute the function).
#
# This is a unit/fixture test — it does not run dnf, does not require
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

echo "--- detect_os() [real function, via OS_RELEASE_FILE fixture]: Amazon Linux 2023 ---"

# A real AL2023 /etc/os-release (trimmed to the fields detect_os() reads).
cat > "$TMPDIR_TEST/os-release-al2023" <<'EOF'
NAME="Amazon Linux"
VERSION="2023"
ID="amzn"
ID_LIKE="fedora"
VERSION_ID="2023"
PLATFORM_ID="platform:al2023"
PRETTY_NAME="Amazon Linux 2023"
EOF

run_detect_os_against() {
  local os_release_file="$1"
  (
    # shellcheck source=/dev/null
    . "$OS_SH"
    OS_RELEASE_FILE="$os_release_file" detect_os
    echo "OS_ID=$OS_ID"
    echo "OS_VERSION_ID=$OS_VERSION_ID"
    echo "OS_PRETTY_NAME=$OS_PRETTY_NAME"
    echo "OS_FAMILY=$OS_FAMILY"
    echo "PKG_MANAGER=$PKG_MANAGER"
    echo "FIREWALL_BACKEND=$FIREWALL_BACKEND"
    echo "OS_SUPPORT=$OS_SUPPORT"
  )
}

out="$(run_detect_os_against "$TMPDIR_TEST/os-release-al2023")"
assert_eq "OS_ID" "amzn" "$(echo "$out" | sed -n 's/^OS_ID=//p')"
assert_eq "OS_FAMILY" "rhel" "$(echo "$out" | sed -n 's/^OS_FAMILY=//p')"
assert_eq "PKG_MANAGER" "dnf" "$(echo "$out" | sed -n 's/^PKG_MANAGER=//p')"
assert_eq "FIREWALL_BACKEND" "firewalld" "$(echo "$out" | sed -n 's/^FIREWALL_BACKEND=//p')"
assert_eq "OS_SUPPORT" "ci-tested" "$(echo "$out" | sed -n 's/^OS_SUPPORT=//p')"

echo
echo "--- detect_os() [real function, via OS_RELEASE_FILE fixture]: generic ID_LIKE=fedora fallback ---"
echo "    (a hypothetical future distro: no explicit amzn/almalinux/rocky/rhel/centos ID,"
echo "     proving the amzn) case and the fallback case are genuinely distinct paths)"

cat > "$TMPDIR_TEST/os-release-generic-fedora-like" <<'EOF'
NAME="Hypothetical Future Linux"
VERSION="1"
ID="hypothetical-future-linux"
ID_LIKE="fedora"
VERSION_ID="1"
PRETTY_NAME="Hypothetical Future Linux 1"
EOF

out2="$(run_detect_os_against "$TMPDIR_TEST/os-release-generic-fedora-like")"
assert_eq "OS_ID (fallback)" "hypothetical-future-linux" "$(echo "$out2" | sed -n 's/^OS_ID=//p')"
assert_eq "OS_FAMILY (fallback)" "rhel" "$(echo "$out2" | sed -n 's/^OS_FAMILY=//p')"
assert_eq "PKG_MANAGER (fallback)" "dnf" "$(echo "$out2" | sed -n 's/^PKG_MANAGER=//p')"
assert_eq "FIREWALL_BACKEND (fallback)" "firewalld" "$(echo "$out2" | sed -n 's/^FIREWALL_BACKEND=//p')"
assert_eq "OS_SUPPORT (fallback)" "untested" "$(echo "$out2" | sed -n 's/^OS_SUPPORT=//p')"

echo
echo "--- detect_os() [real function]: missing OS_RELEASE_FILE fails loudly ---"
missing_rc=0
run_detect_os_against "$TMPDIR_TEST/does-not-exist" >/dev/null 2>&1 || missing_rc=$?
if [ "$missing_rc" -ne 0 ]; then
  echo "ok: detect_os() fails when OS_RELEASE_FILE does not exist"
else
  echo "FAIL: detect_os() did not fail for a missing OS_RELEASE_FILE"
  failures=$((failures + 1))
fi

echo
echo "--- static: deploy/lib/os.sh source structure (supplementary to the functional checks above) ---"
if grep -qE '^\s*amzn\)' "$OS_SH"; then
  echo "ok: os.sh has an explicit 'amzn)' case (not relying on the generic ID_LIKE-fedora fallthrough)"
else
  echo "FAIL: os.sh has no explicit 'amzn)' case"
  failures=$((failures + 1))
fi
if grep -q 'rhel-amzn-2023\*) OS_SUPPORT="ci-tested"' "$OS_SH"; then
  echo "ok: os.sh marks amzn-2023 as ci-tested (not the full 'tested' tier) in the OS_SUPPORT case"
else
  echo "FAIL: os.sh does not mark amzn-2023 as ci-tested"
  failures=$((failures + 1))
fi
if grep -q 'Amazon Linux 2023' "$OS_SH"; then
  echo "ok: os.sh's unsupported-OS error message mentions Amazon Linux 2023"
else
  echo "FAIL: os.sh's unsupported-OS error message does not mention Amazon Linux 2023"
  failures=$((failures + 1))
fi

echo
echo "--- install_dependencies_rhel() [real function body, dnf/systemctl stubbed]: curl-minimal / already-usable-curl handling ---"

# Extract install_dependencies_rhel()'s body from the real install.sh so
# we exercise the actual production logic (eval'd, with dnf/systemctl
# stubbed rather than reimplemented) instead of running real package
# manager commands (no root, no real package manager needed).
INSTALL_DEPS_RHEL_BODY="$(sed -n '/^install_dependencies_rhel() {/,/^}/p' "$INSTALL_SH")"
if [ -z "$INSTALL_DEPS_RHEL_BODY" ]; then
  echo "FAIL: could not extract install_dependencies_rhel() from $INSTALL_SH — has it been renamed/moved?"
  failures=$((failures + 1))
fi

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
