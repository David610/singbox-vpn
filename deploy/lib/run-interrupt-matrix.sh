#!/usr/bin/env bash
# Runs the interruption matrix from an IMMUTABLE copy so editing the repository mid-run cannot affect it,
# and records the harness hash. Usage: run-interrupt-matrix.sh [interrupt_matrix.sh args...]  (default: all)
set -Eeuo pipefail
SRC="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUN="$(mktemp -d /root/run-matrix.XXXXXX)"
cp "$SRC/interrupt_matrix.sh" "$SRC/interrupt_matrix_report.py" "$RUN/"
chmod 0555 "$RUN/interrupt_matrix.sh"; chmod 0444 "$RUN/interrupt_matrix_report.py"
echo "harness copy: $RUN  sha256=$(sha256sum <"$RUN/interrupt_matrix.sh" | cut -c1-16)"
export OUTDIR="${OUTDIR:-/root/evidence/matrix-$(date -u +%Y%m%dT%H%M%S)}"
mkdir -p "$OUTDIR"; cp "$RUN/interrupt_matrix.sh" "$OUTDIR/harness-as-run.sh"
[ "$#" -gt 0 ] || set -- all
exec bash "$RUN/interrupt_matrix.sh" "$@"
