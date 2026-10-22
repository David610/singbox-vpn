#!/usr/bin/env bash
# Runs the Python unit tests (release-evidence validator, Shorts health monitor) (waivers never raise a level,
# commit/tag binding, mandatory device matrix, fingerprint drift).
set -Eeuo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
python3 -I -m unittest discover -v -s . -p "test_*.py"
