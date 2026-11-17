import json
import runpy
from pathlib import Path

root = Path(__file__).resolve().parents[3] / "work" / "phase2"
tool = root / 'r2flutter-main'
generator = runpy.run_path(str(tool / 'scripts/update-dart-version'))
cids = generator['parse_class_id_h'](root / 'dart-class_id.h')
offsets = tool / 'offsets.json'
data = json.loads(offsets.read_text())
config = data['profiles']['3.14.0'] | generator['cid_table_from_cids'](cids)
data['profiles']['3.15.0'] = config
data['hashes']['27c8345752566aecd8ff78f22d05b194'] = '3.15.0'
generator['save_offsets'](offsets, data)
print('Mapped', len(config['cids']), 'class IDs from Dart 3.15.0-77.0.dev.')
