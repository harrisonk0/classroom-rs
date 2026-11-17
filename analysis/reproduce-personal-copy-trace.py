#!/usr/bin/env python3
"""Replay the student submission bootstrap and teacher copy evidence locally."""

import argparse
import json
import os
from pathlib import Path
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("phase2", type=Path, help="workspace work/phase2 directory")
    args = parser.parse_args()
    phase2 = args.phase2.resolve()
    artifact = phase2 / "classroom-arm64.so"
    radare = phase2 / "radare2/Payload/usr/local"
    env = dict(os.environ, DYLD_LIBRARY_PATH=str(radare / "lib"))
    blocks = [
        ("Assignment details invokes the shared submission helper", 0x1947948, 7),
        ("Question details uses the shared submission helper", 0x194B308, 6),
        ("Shared helper invokes the submission loader", 0x193FFE0, 8),
        ("Submission loader builds the query", 0x1ED6790, 8),
        ("Scoped query sets createIfNeeded before dispatch", 0x1940214, 13),
        ("Boolean setter", 0x1795BB4, 4),
        ("QuerySubmissionRequest field 4", 0x197EAB4, 5),
        ("Teacher reuse calls CopyStreamItem", 0x1893D20, 11),
    ]
    for label, address, count in blocks:
        print(label, flush=True)
        subprocess.run(
            [str(radare / "bin/r2"), "-q", "-N", "-e", "scr.color=0",
             "-c", f"pd {count} @ {address:#x}", str(artifact)],
            env=env,
            check=True,
        )

    relevant = {
        "createIfNeeded",
        "shouldCreateMaterialCopies",
        "/homeroom.dataservice.HomeroomDataService/QuerySubmission",
        "/homeroom.dataservice.HomeroomDataService/CopyStreamItem",
    }
    with (phase2 / "exact-xrefs.json").open() as evidence:
        for row in json.load(evidence):
            source = row["src"].get("addr")
            target = row["dst"].get("name")
            if row["kind"] == "code.string" and source is not None and target in relevant:
                print(f"{source:#x} {target}")


if __name__ == "__main__":
    main()
