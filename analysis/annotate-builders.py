import concurrent.futures
import os
from pathlib import Path
import re
import subprocess

root = Path(__file__).resolve().parents[3] / "work" / "phase2"
r2root = root / "radare2/Payload/usr/local"
parser = root / "r2flutter-main/bin/r2flutter"
artifact = root / "classroom-arm64.so"
env = dict(os.environ, DYLD_LIBRARY_PATH=str(r2root / "lib"))
starts = {
    "CourseIdVariantA": 0x196f95c,
    "CourseIdVariantB": 0x1975520,
    "UserFilter": 0x197bb44,
    "QueryCourseRequest": 0x197e1dc,
    "QueryCourseResponse": 0x197e324,
    "Course": 0x1975a0c,
    "CourseReadMask": 0x197eecc,
    "FilterOptions": 0x197b7dc,
    "CourseFilterOptions": 0x197b958,
    "QuerySubmissionRequest": 0x197e9f8,
    "QuerySubmissionResponse": 0x197eb80,
    "StreamItemQueryRequest": 0x1986508,
    "StreamItemQueryResponse": 0x1986658,
}

def resolve(offset):
    output = subprocess.run(
        [parser, "-O", f"pp+{offset:#x}", artifact],
        env=env, capture_output=True, text=True, check=True,
    ).stdout
    values = [line.strip() for line in output.splitlines() if "target.name:" in line or "target.kind:" in line or re.match(r"^  ref=", line)]
    return offset, " | ".join(values)[:500]

blocks = {}
for name, entry in starts.items():
    disassembly = subprocess.run(
        [r2root / "bin/r2", "-N", "-q", "-c", f"e scr.color=0; pd 700 @ {entry:#x}", artifact],
        env=env, capture_output=True, text=True, check=True,
    ).stdout
    registers = {"x27": 0}
    rows = []
    for line in disassembly.splitlines():
        if re.search(r"\sret\s*$", line):
            break
        matched = re.search(r"\badd (x\d+), x27, (0x[0-9a-f]+|\d+), lsl 12\b", line)
        if matched:
            registers[matched[1]] = int(matched[2], 0) << 12
        matched = re.search(r"\bldr (x\d+), \[(x\d+)(?:, (0x[0-9a-f]+|\d+))?\]", line)
        offset = None
        if matched:
            register_offset = registers.get(matched[2])
            if register_offset is not None:
                offset = register_offset + int(matched[3] or '0', 0)
            registers.pop(matched[1], None)
        rows.append((line, offset))
    blocks[name] = rows

offsets = {offset for rows in blocks.values() for _, offset in rows if offset is not None}
with concurrent.futures.ThreadPoolExecutor(max_workers=4) as executor:
    names = dict(executor.map(resolve, sorted(offsets)))
for name, rows in blocks.items():
    path = root / f"builder-{name}.txt"
    with path.open('w') as out:
        for line, offset in rows:
            annotation = '' if offset is None else f"    PP+{offset:#x} {names[offset]}"
            print(line + annotation, file=out)
print(f"Resolved {len(offsets)} pool offsets from {len(blocks)} instruction ranges")
