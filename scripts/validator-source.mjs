// Prepare a pinned binary, or describe the exact production source CI must build.
import fs from 'node:fs';
import path from 'node:path';
import {assert, ROOT, readToml} from './lib/util.mjs';
import {findValidator, fetchValidator} from './lib/validator.mjs';

let binary = findValidator();
if (!binary) binary = await fetchValidator();
const output = process.env.GITHUB_OUTPUT;
assert(output, 'GITHUB_OUTPUT is required');
if (binary) {
  const target = path.join(ROOT, 'tools/bin', process.platform === 'win32' ? 'loom-ext-validate.exe' : 'loom-ext-validate');
  fs.mkdirSync(path.dirname(target), {recursive: true});
  if (binary !== target) fs.copyFileSync(binary, target);
  fs.chmodSync(target, 0o755);
  fs.appendFileSync(output, 'source=false\n');
} else {
  const source = readToml(path.join(ROOT, 'tools/validator-source.toml'));
  assert(source.schema === 1 && /^[\w.-]+\/[\w.-]+$/.test(source.repository) && /^[a-f0-9]{40}$/.test(source.revision), 'Pin the validator repository and complete source commit');
  fs.appendFileSync(output, `source=true\nrepository=${source.repository}\nrevision=${source.revision}\n`);
}
