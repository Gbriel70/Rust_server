#!/usr/bin/env python3
"""Summarize measurement artifacts without running benchmarks or changing code."""
import json
import pathlib
import re

ROOT = pathlib.Path(__file__).resolve().parents[1]
OUT = ROOT / 'validation/phase12'
allocations = []
for case, calls, size in re.findall(r'([^\s:]+): calls/request=(\d+) requested_bytes/request=(\d+)', (OUT/'allocations-after.txt').read_text()):
    allocations.append({'case':case, 'calls':int(calls), 'requested_bytes':int(size)})
results = {'allocations':allocations, 'criterion':json.loads((OUT/'criterion-summary.json').read_text())}
for filename in ['default-test.log','tokio-test.log']:
    text = (OUT/filename).read_text()
    results[filename] = {'passed':sum(map(int,re.findall(r'test result: ok\. (\d+) passed',text))), 'failed_labels':text.count('FAILED')}
text = (OUT/'fuzz-30min.log').read_text()
match = re.search(r'Done (\d+) runs in (\d+) second',text)
results['fuzz'] = {'completed':bool(match), 'runs':int(match[1]) if match else None, 'seconds':int(match[2]) if match else None, 'error_labels':bool(re.search(r'ERROR: libFuzzer|ERROR: AddressSanitizer|runtime error:',text))}
(OUT/'summary.json').write_text(json.dumps(results,indent=2)+'\n')
print(json.dumps(results,indent=2))
