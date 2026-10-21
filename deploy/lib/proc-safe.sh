#!/usr/bin/env bash
# Safe process discovery/termination. Replaces command-line-substring process matching, which matches any process whose
# command line merely CONTAINS the pattern -- including the calling shell, its watcher loop and editors/tails of the same
# name (a controller that killed itself, and "still running" checks that never turned false, both happened).
#
#   proc_pids_exact ERE        PIDs whose full argv (joined by single spaces) matches the ERE in full (anchored here).
#                              Never returns the caller, its ancestors, or PID 1.
#   proc_kill_verified PID ERE [SIG]
#                              signals PID only if it is not the caller/an ancestor/PID 1 and its argv still matches ERE
#                              (guards against PID reuse and wrong targets). Returns 0 sent, 1 refused/not found.
#
# No external dependencies beyond /proc and bash.

_proc_ancestors() { # PIDs of this shell and every ancestor, space separated
  local p="$BASHPID" out="" pp
  while [ -n "$p" ] && [ "$p" -gt 0 ] 2>/dev/null; do
    out="$out $p"
    # comm may contain spaces/parens; take the field after the last ')'
    pp="$(sed 's/.*) //' "/proc/$p/stat" 2>/dev/null | awk '{print $2}')"
    [ -n "$pp" ] && [ "$pp" != "$p" ] && [ "$pp" -gt 0 ] 2>/dev/null || break
    p="$pp"
  done
  echo "$out"
}

_proc_argv() { # empty + rc 1 if the process vanished (races with exit are normal during a /proc scan)
  local out
  out="$({ tr '\0' ' ' <"/proc/$1/cmdline"; } 2>/dev/null)" || return 1
  printf '%s' "${out% }"
}

proc_pids_exact() {
  local re="$1" skip d pid argv
  [ -n "$re" ] || return 2
  skip=" $(_proc_ancestors) 1 "
  for d in /proc/[0-9]*; do
    pid="${d#/proc/}"
    case "$skip" in *" $pid "*) continue;; esac
    argv="$(_proc_argv "$pid")" || continue
    [ -n "$argv" ] || continue
    [[ "$argv" =~ ^($re)$ ]] && echo "$pid"
  done
  return 0
}

proc_kill_verified() {
  local pid="$1" re="$2" sig="${3:-TERM}" skip argv
  [[ "$pid" =~ ^[0-9]+$ ]] && [ -n "$re" ] || return 1
  skip=" $(_proc_ancestors) 1 "
  case "$skip" in *" $pid "*) return 1;; esac
  argv="$(_proc_argv "$pid")" || return 1
  [[ "$argv" =~ ^($re)$ ]] || return 1
  kill -"$sig" "$pid" 2>/dev/null
}
