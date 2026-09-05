#!/usr/bin/env bash
# Shared helper for classifying whether an authenticated (checksum- and
# attestation-verified) release binary actually runs on THIS host, instead
# of collapsing "cannot execute at all" and "executes but the wrong
# version" into the same confusing comparison.
#
# Real-world incident this fixes (v1.0.0-rc.3): the x86_64 release
# binaries were linked against GLIBC_2.39 (the ubuntu-latest build
# runner's own libc) and failed to even start on an AlmaLinux 8 VPS
# (glibc 2.28):
#
#   vpn-admin: /lib64/libc.so.6: version `GLIBC_2.39' not found
#
# The old check was:
#
#   binary_package_version="$("$bin" --version 2>/dev/null | awk '{print $NF}')"
#   [ "$binary_package_version" = "$expected_package_version" ] || die ...
#
# `2>/dev/null` threw away that exact diagnostic, `$bin --version` still
# produced empty stdout, and the failure surfaced as the deeply
# misleading "authenticated binary archive reports version ''" — which
# reads like a packaging/version bug, not a host/ABI incompatibility.
#
# Expects log()/die() to already be defined by the caller (same
# convention as ownership.sh/preflight.sh).
#
# check_binary_version <path> <expected-version> <label> [context-suffix]
#   <path>            executable to run with --version
#   <expected-version> the exact version string expected on the last
#                      whitespace-separated field of --version's stdout
#                      (matches clap's default `<bin-name> <version>`
#                      output for vpn-admin/subscription)
#   <label>            human-readable name for error messages
#                      ("vpn-admin", "subscription")
#   [context-suffix]   optional extra sentence appended to every failure
#                      message (callers use this for "for release
#                      $version." / "Nothing live has been changed.",
#                      keeping each caller's own established wording)
#
# On success, returns 0 with no output. On any of the three failure
# states, calls die() (which is expected to print and exit non-zero) with
# a message that clearly distinguishes:
#   STATE 1 - binary cannot execute (GLIBC mismatch, missing interpreter,
#             exec format error, missing shared library, permissions...)
#   STATE 2 - binary executes but produced no parseable version
#   STATE 3 - binary executes and reports the wrong version (fail closed,
#             same as before)
check_binary_version() {
  local path="$1" expected="$2" label="$3" context="${4:-}"
  local out rc stderr_file excerpt

  stderr_file="$(mktemp)"
  out="$("$path" --version 2>"$stderr_file")"
  rc=$?

  if [ "$rc" -ne 0 ]; then
    # Bounded and newline-collapsed: never let an arbitrarily large or
    # control-character-laden stderr stream reach installer logs
    # unbounded. This is the exact "GLIBC_x.y not found" / "No such file
    # or directory" (missing ELF interpreter) / "cannot execute binary
    # file" (exec format error) / missing-shared-library / permission-
    # denied output that the old `2>/dev/null` silently discarded.
    excerpt="$(tr -d '\000' < "$stderr_file" | tr '\n\r' '  ' | cut -c1-400)"
    rm -f "$stderr_file"
    if [ -n "$excerpt" ]; then
      die "authenticated $label binary cannot execute on this host (exit $rc). This usually means a GLIBC/ELF-interpreter/shared-library mismatch between the release binary and this host, or a permissions problem — not a corrupt download (integrity was already verified). stderr: $excerpt${context:+ $context}"
    else
      die "authenticated $label binary cannot execute on this host (exit $rc, no stderr output). This usually means a GLIBC/ELF-interpreter/shared-library mismatch between the release binary and this host, or a permissions problem — not a corrupt download (integrity was already verified).${context:+ $context}"
    fi
  fi
  rm -f "$stderr_file"

  # Last whitespace-separated token across all of stdout — NOT `awk
  # '{print $NF}'`, which on a whitespace-only (or otherwise zero-field)
  # line prints the whole original line (awk's $NF falls back to $0 when
  # NF is 0), silently turning "blank output" into a bogus non-empty
  # "version". Accumulating into `last` across every field instead
  # leaves it correctly empty when there is no real token anywhere.
  local version
  version="$(printf '%s' "$out" | awk '{for (i = 1; i <= NF; i++) last = $i} END {print last}')"
  if [ -z "$version" ]; then
    local raw_excerpt
    raw_excerpt="$(printf '%s' "$out" | tr '\n\r' '  ' | cut -c1-200)"
    die "authenticated $label binary executed but returned no parseable version (raw --version output: '$raw_excerpt').${context:+ $context}"
  fi

  if [ "$version" != "$expected" ]; then
    die "authenticated $label binary reports version '$version', expected '$expected'.${context:+ $context}"
  fi
}
