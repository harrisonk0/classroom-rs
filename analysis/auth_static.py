#!/usr/bin/env python3
"""Print the bounded authentication evidence from the retained APK analysis."""

from pathlib import Path
import json
import re

workspace = Path(__file__).resolve().parents[3]
phase = workspace / "work/phase2"
smali_root = workspace / "work/static/apktool"
xref = json.loads((phase / "exact-xrefs.json").read_text())
strings = json.loads((phase / "arm64-strings.json").read_text())
public_markers = {
    "scopes",
    "plugins.flutter.io/sso_auth",
    "https://www.googleapis.com/auth/homeroom",
    "https://www.googleapis.com/auth/drive",
    "https://www.googleapis.com/auth/plus.peopleapi.readwrite",
    "Authorization",
    "Bearer ",
    "X-Goog-AuthUser",
    "x-goog-api-client",
    "x-goog-ext-525005006-bin",
    "x-goog-ext-244371700-bin",
    "x-goog-ext-53200201-bin",
    "classroomRoutingInfo",
    "compass.RoutingInfo",
}
markers = re.compile(
    r"play[._/ -]?integrity|IntegrityManager|standardIntegrity|integrityToken|"
    r"request.?integrity.?token|playintegrity\.googleapis\.com|"
    r"setAttestationChallenge|SafetyNet|DroidGuard|"
    r"\bdpop\b|device.?bound|x-goog-attest",
    re.I,
)

print("Allowed string references in the ARM64 Dart code")
records = sorted({
    (row["src"]["addr"], row["dst"]["name"])
    for row in xref
    if row["kind"] == "code.string" and row["dst"]["name"] in public_markers
})
for address, value in records:
    print(f"{address:#010x} {value!r}")

print("\nBounded attestation and token-binding marker scan")
found_snapshot = sorted({
    (item["addr"], match.group(0))
    for item in strings
    for match in markers.finditer(item["value"])
})
for address, marker in found_snapshot:
    print(f"Dart {address:#010x} {marker}")
if not found_snapshot:
    print("No matching markers in the carved Dart string inventory.")

files = [
    smali_root / "AndroidManifest.xml",
    *(path for directory in sorted(smali_root.glob("smali*")) for path in directory.rglob("*.smali")),
]
for file in files:
    for number, line in enumerate(file.read_text(errors="replace").splitlines(), start=1):
        for match in markers.finditer(line):
            print(f"{file.relative_to(workspace)}:{number} {match.group(0)}")
print("This scan cannot rule out checks in Google Play Services or Google's servers.")
