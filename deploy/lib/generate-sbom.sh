#!/usr/bin/env bash
# Release SBOM + license inventory (SEC-005) for the exact release candidate.
#
#   bash deploy/lib/generate-sbom.sh --version vX.Y.Z --output-dir DIR
#       [--metadata-json FILE]   (test hook: use a saved `cargo metadata`)
#
# Writes, deterministically (same commit + Cargo.lock => same bytes):
#   DIR/singbox-vpn-sbom.cdx.json      CycloneDX 1.5 JSON
#   DIR/singbox-vpn-licenses.json      per-package license inventory
#
# Scope: every crate in the RUNTIME dependency graph of the two shipped
# binaries (vpn-admin from the `admin` package, `subscription`), resolved
# for x86_64-unknown-linux-gnu with `cargo metadata --locked` so it can only
# describe the committed Cargo.lock. Crate hashes are the registry SHA-256
# checksums recorded in Cargo.lock. The pinned upstream sing-box binary the
# installer downloads (deploy/lib/versions.env) is listed as an external
# component with its pinned SHA-256; it is not inside the binary archive.
#
# Fails closed: a package with neither `license` nor `license_file`, an
# unlocked/unresolvable graph, or a missing shipped binary package aborts
# the release instead of publishing an incomplete inventory.
set -Eeuo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
VERSION=""
OUTPUT_DIR=""
METADATA_JSON=""

while [ $# -gt 0 ]; do
  case "$1" in
    --version) VERSION="${2:-}"; shift 2 ;;
    --output-dir) OUTPUT_DIR="${2:-}"; shift 2 ;;
    --metadata-json) METADATA_JSON="${2:-}"; shift 2 ;;
    *) echo "generate-sbom: unknown argument: $1" >&2; exit 2 ;;
  esac
done
[ -n "$VERSION" ] || { echo "generate-sbom: --version is required" >&2; exit 2; }
[ -n "$OUTPUT_DIR" ] || { echo "generate-sbom: --output-dir is required" >&2; exit 2; }
command -v python3 >/dev/null 2>&1 || { echo "generate-sbom: python3 is required" >&2; exit 1; }
mkdir -p "$OUTPUT_DIR"

metadata_file="$(mktemp)"
trap 'rm -f "$metadata_file"' EXIT
if [ -n "$METADATA_JSON" ]; then
  cp "$METADATA_JSON" "$metadata_file"
else
  (cd "$REPO_ROOT" && cargo metadata --format-version 1 --locked \
    --filter-platform x86_64-unknown-linux-gnu) >"$metadata_file"
fi

commit="$(git -C "$REPO_ROOT" rev-parse HEAD 2>/dev/null || echo unknown)"
epoch="${SOURCE_DATE_EPOCH:-$(git -C "$REPO_ROOT" log -1 --format=%ct 2>/dev/null || echo 0)}"
# shellcheck source=/dev/null
. "$REPO_ROOT/deploy/lib/versions.env"

python3 - "$metadata_file" "$REPO_ROOT/Cargo.lock" "$OUTPUT_DIR" "$VERSION" "$commit" "$epoch" \
  "$SINGBOX_VERSION" "$SINGBOX_SHA256_AMD64" <<'PY'
import datetime, hashlib, json, sys, uuid

(metadata_path, lock_path, out_dir, version, commit, epoch,
 singbox_version, singbox_sha256) = sys.argv[1:9]
metadata = json.load(open(metadata_path, encoding="utf-8"))
lock_text = open(lock_path, encoding="utf-8").read()

checksums = {}
name = ver = None
for line in lock_text.splitlines():
    line = line.strip()
    if line == "[[package]]":
        name = ver = None
    elif line.startswith("name = "):
        name = line.split("=", 1)[1].strip().strip('"')
    elif line.startswith("version = "):
        ver = line.split("=", 1)[1].strip().strip('"')
    elif line.startswith("checksum = ") and name and ver:
        checksums[(name, ver)] = line.split("=", 1)[1].strip().strip('"')

packages = {p["id"]: p for p in metadata["packages"]}
nodes = {n["id"]: n for n in metadata["resolve"]["nodes"]}
workspace = set(metadata["workspace_members"])

shipped = [pid for pid in workspace if packages[pid]["name"] in ("admin", "subscription")]
if len(shipped) != 2:
    sys.exit("generate-sbom: could not find both shipped packages (admin, subscription)")

runtime, stack, edges = set(), list(shipped), {}
while stack:
    pid = stack.pop()
    if pid in runtime:
        continue
    runtime.add(pid)
    deps = []
    for dep in nodes[pid]["deps"]:
        if any(kind.get("kind") is None for kind in dep["dep_kinds"]):
            deps.append(dep["pkg"])
            stack.append(dep["pkg"])
    edges[pid] = sorted(set(deps))

def purl(p):
    return f"pkg:cargo/{p['name']}@{p['version']}"

components, inventory, missing = [], [], []
for pid in sorted(runtime, key=lambda i: (packages[i]["name"], packages[i]["version"])):
    p = packages[pid]
    license_expr = p.get("license")
    if not license_expr and not p.get("license_file"):
        missing.append(purl(p))
    component = {
        "type": "application" if pid in shipped else "library",
        "bom-ref": purl(p),
        "name": p["name"],
        "version": p["version"],
        "purl": purl(p),
        "scope": "required",
    }
    if license_expr:
        component["licenses"] = [{"expression": license_expr}]
    elif p.get("license_file"):
        component["licenses"] = [{"license": {"name": "LicenseRef-file"}}]
    checksum = checksums.get((p["name"], p["version"]))
    if checksum:
        component["hashes"] = [{"alg": "SHA-256", "content": checksum}]
    source = p.get("source") or ("workspace" if pid in workspace else "path")
    component["properties"] = [{"name": "cargo:source", "value": source}]
    components.append(component)
    inventory.append({
        "name": p["name"],
        "version": p["version"],
        "purl": purl(p),
        "license": license_expr,
        "license_file": bool(p.get("license_file")),
        "source": source,
        "workspace_member": pid in workspace,
    })

if missing:
    sys.exit("generate-sbom: packages without license metadata: " + ", ".join(missing))

singbox_ref = f"pkg:github/SagerNet/sing-box@v{singbox_version}"
components.append({
    "type": "application",
    "bom-ref": singbox_ref,
    "name": "sing-box",
    "version": singbox_version,
    "purl": singbox_ref,
    "scope": "required",
    "licenses": [{"expression": "GPL-3.0-or-later"}],
    "hashes": [{"alg": "SHA-256", "content": singbox_sha256}],
    "properties": [
        {"name": "singbox-vpn:delivery", "value": "downloaded and checksum-verified by the installer; not inside the binary archive"},
        {"name": "singbox-vpn:asset", "value": f"sing-box-{singbox_version}-linux-amd64.tar.gz"},
    ],
})

root_ref = f"pkg:github/David610/singbox-vpn@{version}"
dependencies = [{"ref": root_ref, "dependsOn": sorted([purl(packages[p]) for p in shipped] + [singbox_ref])}]
for pid in sorted(edges, key=lambda i: purl(packages[i])):
    dependencies.append({"ref": purl(packages[pid]), "dependsOn": sorted({purl(packages[d]) for d in edges[pid]})})
dependencies.append({"ref": singbox_ref, "dependsOn": []})

timestamp = datetime.datetime.fromtimestamp(int(epoch), datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
fingerprint = hashlib.sha256((version + commit + lock_text).encode()).hexdigest()
bom = {
    "bomFormat": "CycloneDX",
    "specVersion": "1.5",
    "serialNumber": f"urn:uuid:{uuid.UUID(fingerprint[:32], version=4)}",
    "version": 1,
    "metadata": {
        "timestamp": timestamp,
        "component": {
            "type": "application",
            "bom-ref": root_ref,
            "name": "singbox-vpn",
            "version": version,
            "purl": root_ref,
            "licenses": [{"expression": "Apache-2.0"}],
        },
        "properties": [
            {"name": "singbox-vpn:commit", "value": commit},
            {"name": "singbox-vpn:cargo-lock-sha256", "value": hashlib.sha256(lock_text.encode()).hexdigest()},
            {"name": "singbox-vpn:target", "value": "x86_64-unknown-linux-gnu"},
        ],
    },
    "components": components,
    "dependencies": dependencies,
}

summary = {}
for item in inventory:
    key = item["license"] or "LicenseRef-file"
    summary[key] = summary.get(key, 0) + 1
licenses = {
    "release": version,
    "commit": commit,
    "cargo_lock_sha256": bom["metadata"]["properties"][1]["value"],
    "scope": "runtime dependency graph of vpn-admin and subscription (x86_64-unknown-linux-gnu)",
    "packages": inventory,
    "external_components": [{"name": "sing-box", "version": singbox_version, "license": "GPL-3.0-or-later", "sha256": singbox_sha256}],
    "license_summary": dict(sorted(summary.items())),
}

for filename, doc in (("singbox-vpn-sbom.cdx.json", bom), ("singbox-vpn-licenses.json", licenses)):
    with open(f"{out_dir}/{filename}", "w", encoding="utf-8") as handle:
        json.dump(doc, handle, indent=2, sort_keys=True)
        handle.write("\n")
print(f"generate-sbom: {len(components)} components, {len(summary)} distinct license expressions")
PY
