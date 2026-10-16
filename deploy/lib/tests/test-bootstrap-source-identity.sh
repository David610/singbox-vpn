#!/usr/bin/env bash
# I-16: the development-channel bootstrap must deliver exactly the revision it
# was asked for, and tell the deployment installer which commit that was.
# Executes the real install.sh against a fake codeload surface and a stand-in
# deployment installer. Linux only (the bootstrap refuses other hosts).
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
BOOTSTRAP="$REPO_ROOT/install.sh"

if [ ! -f /etc/os-release ] || ! command -v git >/dev/null 2>&1; then
  echo "bootstrap source identity: SKIP (needs Linux and git)"
  exit 0
fi

TEST_TMP="$(mktemp -d)"
trap 'rm -rf "$TEST_TMP"' EXIT
failures=0
ok() { echo "ok: $*"; }
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }

# A real git commit, archived the way GitHub's codeload does (the commit id is
# embedded in the tar's pax global header).
SRC="$TEST_TMP/repo"
mkdir -p "$SRC/deploy/almalinux"
cat >"$SRC/deploy/almalinux/install.sh" <<'INSTALLER'
#!/usr/bin/env bash
set -Eeuo pipefail
printf 'commit=%s ref=%s\n' "${SINGBOX_VPN_SOURCE_COMMIT:-}" "${SINGBOX_VPN_SOURCE_REF:-}" >"$TEST_MARKER"
INSTALLER
chmod 0755 "$SRC/deploy/almalinux/install.sh"
printf '[workspace.package]\nversion = "0.1.3"\n' >"$SRC/Cargo.toml"
# Real archives are megabytes: the commit id must still be read when gzip is
# killed by SIGPIPE after the header has been consumed (pipefail regression).
head -c 3145728 /dev/urandom >"$SRC/padding.bin"
git -C "$SRC" init -q
git -C "$SRC" -c user.email=t@example.test -c user.name=t add -A
git -C "$SRC" -c user.email=t@example.test -c user.name=t commit -q -m fixture
COMMIT="$(git -C "$SRC" rev-parse HEAD)"
git -C "$SRC" archive --format=tar.gz --prefix=singbox-vpn-fixture/ -o "$TEST_TMP/archive.tar.gz" HEAD

mkdir -p "$TEST_TMP/fakebin"
cat >"$TEST_TMP/fakebin/id" <<'FAKE_ID'
#!/usr/bin/env bash
if [ "${1:-}" = "-u" ]; then echo 0; else exec /usr/bin/id "$@"; fi
FAKE_ID
cat >"$TEST_TMP/fakebin/curl" <<'FAKE_CURL'
#!/usr/bin/env bash
set -Eeuo pipefail
out="" url=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o) out="$2"; shift 2 ;;
    --connect-timeout|--max-time|--speed-limit|--speed-time|--retry|--retry-delay) shift 2 ;;
    -*) shift ;;
    *) url="$1"; shift ;;
  esac
done
echo "$url" >>"$CURL_LOG"
case "$url" in
  https://codeload.github.com/*/tar.gz/*) cp "$FIXTURE_TAR" "$out" ;;
  *) echo "unexpected curl URL: $url" >&2; exit 1 ;;
esac
FAKE_CURL
chmod 0755 "$TEST_TMP/fakebin/id" "$TEST_TMP/fakebin/curl"

export FIXTURE_TAR="$TEST_TMP/archive.tar.gz" TEST_MARKER="$TEST_TMP/marker" CURL_LOG="$TEST_TMP/curl.log"

run_bootstrap() {
  : >"$TEST_MARKER"
  : >"$CURL_LOG"
  PATH="$TEST_TMP/fakebin:$PATH" SINGBOX_VPN_CHANNEL=dev SINGBOX_VPN_ALLOW_UNVERIFIED_DEV=1 \
    bash "$BOOTSTRAP" "$@" >"$TEST_TMP/out.log" 2>&1
}

if run_bootstrap --ref "$COMMIT" && grep -qx "commit=$COMMIT ref=$COMMIT" "$TEST_MARKER" \
    && grep -q "/tar.gz/$COMMIT\$" "$CURL_LOG"; then
  ok "a pinned commit is fetched by SHA and its identity is handed to the installer"
else
  fail "pinned commit: $(cat "$TEST_MARKER") :: $(cat "$CURL_LOG") :: $(tail -5 "$TEST_TMP/out.log")"
fi

OTHER="0000000000000000000000000000000000000001"
if run_bootstrap --ref "$OTHER"; then
  fail "an archive of a different commit than requested was accepted"
elif grep -q "refusing to build a different revision" "$TEST_TMP/out.log" && [ ! -s "$TEST_MARKER" ]; then
  ok "an archive whose commit differs from the requested SHA is refused before any hand-off"
else
  fail "wrong-commit refusal: $(tail -5 "$TEST_TMP/out.log")"
fi

if run_bootstrap --ref main && grep -qx "commit=$COMMIT ref=main" "$TEST_MARKER" \
    && grep -q "/tar.gz/refs/heads/main\$" "$CURL_LOG"; then
  ok "a branch ref records the commit the branch resolved to"
else
  fail "branch ref: $(cat "$TEST_MARKER") :: $(tail -5 "$TEST_TMP/out.log")"
fi

for bad in '../etc' 'main;id' 'a b'; do
  if run_bootstrap --ref "$bad"; then
    fail "malformed ref '$bad' was accepted"
  elif grep -q "invalid --ref" "$TEST_TMP/out.log" && [ ! -s "$CURL_LOG" ]; then
    ok "malformed ref '$bad' is refused before any download"
  else
    fail "malformed ref '$bad': $(tail -3 "$TEST_TMP/out.log")"
  fi
done

[ "$failures" -eq 0 ] || { echo "$failures failure(s)"; exit 1; }
echo "bootstrap source identity checks passed"
