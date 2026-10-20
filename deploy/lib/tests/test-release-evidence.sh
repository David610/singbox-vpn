#!/usr/bin/env bash
# Runs the release-evidence validator's unit tests (waivers never raise a level,
# commit/tag binding, mandatory device matrix, fingerprint drift).
set -Eeuo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
python3 -I -m unittest discover -v -s . -p "test_release_evidence.py"
