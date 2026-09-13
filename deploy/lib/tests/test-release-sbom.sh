#!/usr/bin/env bash
# SEC-005: every release ships an SBOM and a license inventory for the exact
# release candidate. Static checks keep release.yml from silently dropping
# them; functional checks exercise the real generator on a fixture
# `cargo metadata` (no cargo/network needed) for determinism and
# fail-closed license handling. CI's `test` job additionally runs the
# generator against the real workspace.
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
RELEASE_YML="$REPO_ROOT/.github/workflows/release.yml"
CI_YML="$REPO_ROOT/.github/workflows/ci.yml"
GENERATOR="$REPO_ROOT/deploy/lib/generate-sbom.sh"

failures=0
ok() { echo "ok: $*"; }
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

echo "--- static: release.yml generates, attests, uploads and publishes the SBOM + license inventory ---"
build_job="$(sed -n '/^  build:$/,/^  runtime-compat:$/p' "$RELEASE_YML")"
publish_job="$(sed -n '/^  publish:$/,/^  verify-published-bootstrap:$/p' "$RELEASE_YML")"
echo "$build_job" | grep -q 'bash deploy/lib/generate-sbom.sh --version "\$RELEASE_TAG"' \
  && ok "build job runs the generator for the release tag" || fail "build job does not generate the SBOM"
build_line="$(echo "$build_job" | grep -n 'build-release-x86_64.sh' | head -1 | cut -d: -f1)"
sbom_line="$(echo "$build_job" | grep -n 'generate-sbom.sh' | head -1 | cut -d: -f1)"
[ -n "$build_line" ] && [ -n "$sbom_line" ] && [ "$build_line" -lt "$sbom_line" ] \
  && ok "SBOM is generated in the same job, after the release binaries are built" \
  || fail "SBOM generation is not in the build job after the release build"
echo "$build_job" | grep -A8 'Attest SBOM and license inventory provenance' | grep -q 'singbox-vpn-sbom.cdx.json' \
  && ok "SBOM is covered by a Sigstore provenance attestation" || fail "SBOM is not attested"
for f in singbox-vpn-sbom.cdx.json singbox-vpn-licenses.json singbox-vpn-sbom.sigstore.json; do
  echo "$build_job" | grep -q "^            $f$" && ok "build uploads $f" || fail "build does not upload $f"
done
for f in singbox-vpn-sbom.cdx.json singbox-vpn-licenses.json; do
  echo "$publish_job" | grep -q "dist/$f" && ok "publish ships $f" || fail "publish does not ship $f"
  echo "$publish_job" | grep -q "test -s $f" && ok "publish refuses to run without $f" || fail "publish does not require $f"
  echo "$publish_job" | grep -q "$f.sha256" && ok "SHA256SUMS covers $f" || fail "SHA256SUMS does not cover $f"
done
echo "$publish_job" | grep -q 'dist/\*.sigstore.json' && ok "publish still ships Sigstore bundles" || fail "publish lost the Sigstore bundles"
grep -q 'generate-sbom.sh --version ci-dry-run' "$CI_YML" \
  && ok "CI runs the generator against the real workspace on every change" || fail "CI does not dry-run the SBOM generator"

echo
echo "--- functional: generator on a fixture metadata graph ---"
cat >"$WORK/metadata.json" <<'EOF'
{
  "workspace_members": ["admin 1.0.0 (path+file:///w/apps/admin)", "subscription 1.0.0 (path+file:///w/services/subscription)", "compat-config 1.0.0 (path+file:///w/crates/compat-config)"],
  "packages": [
    {"id": "admin 1.0.0 (path+file:///w/apps/admin)", "name": "admin", "version": "1.0.0", "license": "Apache-2.0", "license_file": null, "source": null},
    {"id": "subscription 1.0.0 (path+file:///w/services/subscription)", "name": "subscription", "version": "1.0.0", "license": "Apache-2.0", "license_file": null, "source": null},
    {"id": "compat-config 1.0.0 (path+file:///w/crates/compat-config)", "name": "compat-config", "version": "1.0.0", "license": "Apache-2.0", "license_file": null, "source": null},
    {"id": "serde 1.0.0 (registry+https://github.com/rust-lang/crates.io-index)", "name": "serde", "version": "1.0.0", "license": "MIT OR Apache-2.0", "license_file": null, "source": "registry+https://github.com/rust-lang/crates.io-index"},
    {"id": "tempfile 3.0.0 (registry+https://github.com/rust-lang/crates.io-index)", "name": "tempfile", "version": "3.0.0", "license": "MIT OR Apache-2.0", "license_file": null, "source": "registry+https://github.com/rust-lang/crates.io-index"}
  ],
  "resolve": {"nodes": [
    {"id": "admin 1.0.0 (path+file:///w/apps/admin)", "deps": [
      {"pkg": "compat-config 1.0.0 (path+file:///w/crates/compat-config)", "dep_kinds": [{"kind": null}]},
      {"pkg": "tempfile 3.0.0 (registry+https://github.com/rust-lang/crates.io-index)", "dep_kinds": [{"kind": "dev"}]}
    ]},
    {"id": "subscription 1.0.0 (path+file:///w/services/subscription)", "deps": [
      {"pkg": "compat-config 1.0.0 (path+file:///w/crates/compat-config)", "dep_kinds": [{"kind": null}]}
    ]},
    {"id": "compat-config 1.0.0 (path+file:///w/crates/compat-config)", "deps": [
      {"pkg": "serde 1.0.0 (registry+https://github.com/rust-lang/crates.io-index)", "dep_kinds": [{"kind": null}]}
    ]},
    {"id": "serde 1.0.0 (registry+https://github.com/rust-lang/crates.io-index)", "deps": []},
    {"id": "tempfile 3.0.0 (registry+https://github.com/rust-lang/crates.io-index)", "deps": []}
  ]}
}
EOF
export SOURCE_DATE_EPOCH=1700000000
if bash "$GENERATOR" --version v9.9.9-test --output-dir "$WORK/a" --metadata-json "$WORK/metadata.json" >/dev/null \
    && bash "$GENERATOR" --version v9.9.9-test --output-dir "$WORK/b" --metadata-json "$WORK/metadata.json" >/dev/null; then
  ok "generator succeeds on a fully licensed graph"
else
  fail "generator failed on a fully licensed graph"
fi
if cmp -s "$WORK/a/singbox-vpn-sbom.cdx.json" "$WORK/b/singbox-vpn-sbom.cdx.json" \
    && cmp -s "$WORK/a/singbox-vpn-licenses.json" "$WORK/b/singbox-vpn-licenses.json"; then
  ok "output is byte-for-byte deterministic"
else
  fail "two runs over the same inputs produced different bytes"
fi
python3 - "$WORK/a" <<'PY' && ok "SBOM is CycloneDX 1.5 with the runtime graph only, hashes from Cargo.lock, and sing-box pinned" || fail "SBOM content is wrong"
import json, sys
d = sys.argv[1]
bom = json.load(open(f"{d}/singbox-vpn-sbom.cdx.json"))
lic = json.load(open(f"{d}/singbox-vpn-licenses.json"))
assert bom["bomFormat"] == "CycloneDX" and bom["specVersion"] == "1.5"
assert bom["metadata"]["component"]["version"] == "v9.9.9-test"
assert bom["metadata"]["timestamp"] == "2023-11-14T22:13:20Z"
names = sorted(c["name"] for c in bom["components"])
assert names == ["admin", "compat-config", "serde", "sing-box", "subscription"], names
assert "tempfile" not in json.dumps(lic), "dev-only dependency leaked into the runtime inventory"
singbox = next(c for c in bom["components"] if c["name"] == "sing-box")
assert singbox["hashes"][0]["alg"] == "SHA-256" and len(singbox["hashes"][0]["content"]) == 64
assert all("licenses" in c for c in bom["components"])
assert lic["license_summary"]["MIT OR Apache-2.0"] == 1
PY
sed 's/"license": "MIT OR Apache-2.0", "license_file": null, "source": "registry+https:\/\/github.com\/rust-lang\/crates.io-index"}, *$/"license": null, "license_file": null, "source": "registry"},/' \
  "$WORK/metadata.json" | python3 -c '
import json, sys
m = json.load(sys.stdin)
for p in m["packages"]:
    if p["name"] == "serde":
        p["license"] = None
json.dump(m, sys.stdout)' >"$WORK/unlicensed.json"
if bash "$GENERATOR" --version v9.9.9-test --output-dir "$WORK/c" --metadata-json "$WORK/unlicensed.json" >/dev/null 2>"$WORK/c.err"; then
  fail "generator accepted a runtime dependency without license metadata"
else
  grep -q 'packages without license metadata: pkg:cargo/serde@1.0.0' "$WORK/c.err" \
    && ok "a runtime dependency without license metadata fails the release closed" \
    || fail "unexpected generator error: $(cat "$WORK/c.err")"
fi

echo
if [ "$failures" -ne 0 ]; then
  echo "$failures release SBOM test(s) failed"
  exit 1
fi
echo "all release SBOM tests passed"
