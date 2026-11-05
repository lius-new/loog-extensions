// Prepare a pinned binary, or describe the exact production source CI must build.
import fs from 'node:fs';
import path from 'node:path';
import {assert, ROOT} from './lib/util.mjs';
import {findValidator, fetchValidator} from './lib/validator.mjs';
import {readLoomSource} from './lib/loom-source.mjs';

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
  const {repository, revision} = readLoomSource();
  fs.appendFileSync(output, `source=true\nrepository=${repository}\nrevision=${revision}\n`);
}
