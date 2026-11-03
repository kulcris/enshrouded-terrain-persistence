"""Verify the production commit signature against supplied PE executables."""
import json, re, sys
from pathlib import Path
import pefile

source = Path('src/native.rs').read_text()
pattern = bytes.fromhex(re.search(r'COMMIT_PATTERN: &str = "([^"]+)"', source)[1])
results = []
for filename in sys.argv[1:]:
    pe = pefile.PE(filename, fast_load=True)
    found = []
    for section in pe.sections:
        if not section.Characteristics & 0x20000000:
            continue
        data = section.get_data()
        start = 0
        while (start := data.find(pattern, start)) >= 0:
            found.append(section.VirtualAddress + start)
            start += 1
    if len(found) != 1:
        raise SystemExit(f'{filename}: expected one signature, found {len(found)}')
    results.append({'file': filename, 'unique_commit_rva': hex(found[0])})
print(json.dumps(results, indent=2))
