#!/usr/bin/env bash
# Release-pipeline gate: fail if any given Linux ELF binary requires a
# GLIBC_x.y[.z] symbol version newer than the declared baseline.
#
# Real-world incident this exists to catch (v1.0.0-rc.3): the x86_64
# release binaries were built on ubuntu-latest and inherited its glibc
# 2.39 ABI, so they failed to even start on an AlmaLinux 8 VPS (glibc
# 2.28) — and, since AlmaLinux 9 (the actual v1.0 SUPPORTED TARGET) only
# ships glibc 2.34, they would have failed there too. The old release
# pipeline's "archive/installer contract test" only ran `--version` on
# whichever architecture the CI runner itself happened to be (i.e. the
# same/newer glibc that built it), so it could never have caught this.
#
# What actually determines the minimum glibc a dynamically linked
# binary can run against is the set of versioned symbols (GLIBC_x.y
# tags) it imports from libc.so.6 — NOT what glibc the compiler/rustc
# itself needed, and NOT the target triple. `objdump -T` lists the
# dynamic symbol table including those version tags.
#
# The comparison below is numeric, component-by-component
# (major, then minor) — never lexicographic/string comparison, which
# would get "2.9" vs "2.28" backwards (lexicographically "2.9" > "2.28",
# but 2.9 < 2.28).
#
# Usage:
#   check-glibc-baseline.sh <baseline e.g. 2.28> <binary> [<binary> ...]
#
# Exits non-zero (after checking and reporting on every binary given,
# not stopping at the first failure) if any binary's maximum required
# GLIBC_* version exceeds the baseline, or if a binary could not be
# analyzed at all.
set -uo pipefail

# ---------------------------------------------------------------------
# Pure logic (no ELF parsing) — kept separate from CLI/objdump plumbing
# below so deploy/lib/tests/test-glibc-baseline-check.sh can source this
# file and exercise the actual comparison against fabricated version
# lists, without needing binaries built against every glibc version.
# ---------------------------------------------------------------------

# glibc_version_le <version> <baseline>
# True (rc 0) iff <version> <= <baseline>, compared numerically by
# major then minor component (a trailing patch component, e.g. the rare
# "GLIBC_2.2.5", is ignored — glibc versions this baseline cares about
# are compared at major.minor granularity, matching how symbol versions
# are actually cut upstream).
glibc_version_le() {
  local version="$1" baseline="$2"
  local v_major v_minor b_major b_minor
  IFS='.' read -r v_major v_minor _ <<<"$version"
  IFS='.' read -r b_major b_minor _ <<<"$baseline"
  if [ "$v_major" -lt "$b_major" ]; then return 0; fi
  if [ "$v_major" -gt "$b_major" ]; then return 1; fi
  [ "$v_minor" -le "$b_minor" ]
}

# glibc_max_version <version...>
# Prints the numerically-greatest version from the given list (major.minor).
glibc_max_version() {
  local max_major=-1 max_minor=-1 max_str="" v major minor
  for v in "$@"; do
    IFS='.' read -r major minor _ <<<"$v"
    if [ "$major" -gt "$max_major" ] || { [ "$major" -eq "$max_major" ] && [ "$minor" -gt "$max_minor" ]; }; then
      max_major="$major" max_minor="$minor" max_str="$v"
    fi
  done
  printf '%s' "$max_str"
}

is_valid_glibc_version() {
  [[ "$1" =~ ^[0-9]+\.[0-9]+(\.[0-9]+)?$ ]]
}

# ---------------------------------------------------------------------
# ELF inspection + CLI. Guarded so sourcing this file for its pure
# functions above (the unit test) never runs this against argv.
# ---------------------------------------------------------------------
glibc_versions_from_binary() {
  local bin="$1"
  objdump -T "$bin" 2>/dev/null | grep -oE 'GLIBC_[0-9]+\.[0-9]+(\.[0-9]+)?' | sed 's/^GLIBC_//' | sort -u
}

check_one_binary() {
  local bin="$1" baseline="$2"
  echo "=== $bin ==="
  if [ ! -f "$bin" ]; then
    echo "::error::$bin: no such file"
    return 1
  fi
  command -v file >/dev/null 2>&1 && { echo "--- file ---"; file "$bin"; }
  echo "--- readelf -d (dynamic section) ---"
  readelf -d "$bin" 2>/dev/null || echo "(readelf -d produced no output)"
  echo "--- readelf --version-info ---"
  readelf --version-info "$bin" 2>/dev/null || echo "(readelf --version-info produced no output)"

  local versions=()
  while IFS= read -r line; do
    [ -n "$line" ] && versions+=("$line")
  done < <(glibc_versions_from_binary "$bin")

  if [ "${#versions[@]}" -eq 0 ]; then
    echo "::error::$bin: found no GLIBC_* version symbols at all in its dynamic symbol table (objdump -T) — this is unexpected for a dynamically linked glibc binary and most likely means objdump/parsing failed, not that the binary has no glibc dependency. Refusing to treat an unanalyzable binary as passing."
    return 1
  fi

  local max_version
  max_version="$(glibc_max_version "${versions[@]}")"
  echo "detected required GLIBC_* versions: ${versions[*]}"
  echo "maximum required: GLIBC_$max_version"

  if glibc_version_le "$max_version" "$baseline"; then
    echo "ok: $bin requires at most GLIBC_$max_version, within the GLIBC_$baseline release baseline."
    return 0
  fi
  echo "::error::$bin requires GLIBC_$max_version; release baseline is GLIBC_$baseline"
  return 1
}

main() {
  if [ "$#" -lt 2 ]; then
    echo "usage: $0 <max-glibc-baseline e.g. 2.28> <binary> [<binary> ...]" >&2
    exit 2
  fi
  local baseline="$1"; shift
  if ! is_valid_glibc_version "$baseline"; then
    echo "::error::invalid baseline version '$baseline' (expected e.g. 2.28)" >&2
    exit 2
  fi
  for tool in objdump readelf; do
    command -v "$tool" >/dev/null 2>&1 || { echo "::error::required tool '$tool' not found on PATH" >&2; exit 2; }
  done

  local overall_rc=0
  local bin
  for bin in "$@"; do
    check_one_binary "$bin" "$baseline" || overall_rc=1
    echo
  done
  exit "$overall_rc"
}

if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
  main "$@"
fi
