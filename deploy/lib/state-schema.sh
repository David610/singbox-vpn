#!/usr/bin/env bash
# Shared `vpn-admin config validate`/`config migrate` wiring, shared by
# deploy/almalinux/install.sh (check_state_schema()) and
# deploy/almalinux/update.sh (both its production and --dev-rebuild
# paths) -- extracted after Phase 5 of the v1.0 completion program added
# a third near-identical copy of this exact validate/migrate/branch
# block to update.sh. Sourced, not executed; expects log() to already be
# defined by the caller (die() is deliberately NOT called from here --
# see below).
#
# $1 = vpn-admin binary path, $2 = deployment.toml path.
#
# Returns (never calls die() itself, so every failure keeps its own
# call site's precise, context-specific recovery message -- install.sh
# and update.sh describe very different rollback stories for the same
# failure):
#   0 = schema already current, no migration needed
#   2 = MIGRATION_REQUIRED was detected and `config migrate` succeeded
#   3 = MIGRATION_REQUIRED was detected but `config migrate` failed
#       (validate's own output was already echoed to stdout first)
#   1 = schema is invalid/unsupported (validate's output already printed
#       to stderr)
state_schema_validate_and_migrate() {
  local admin_bin="$1" deployment_toml="$2"
  local validate_output="" validate_rc=0
  validate_output="$("$admin_bin" --config "$deployment_toml" config validate 2>&1)" || validate_rc=$?
  case "$validate_rc" in
    0)
      log "persistent state schema: current, no migration needed."
      return 0
      ;;
    2)
      log "persistent state schema: MIGRATION REQUIRED. Migrating now (backup taken automatically before any change)..."
      echo "$validate_output"
      if "$admin_bin" --config "$deployment_toml" config migrate; then
        log "persistent state schema: migration complete."
        return 2
      else
        return 3
      fi
      ;;
    *)
      echo "$validate_output" >&2
      echo "(vpn-admin config validate exited $validate_rc)" >&2
      return 1
      ;;
  esac
}
