// `loom-ext-validate` is built by Loom's own repository, from the same loader and
// query compilers the application uses, and published with each Loom release. This
// repository only runs it: `loom-ext-validate <extension-folder> [--samples file]`
// prints one JSON object per grammar (`{"grammar": "rust", "colored": 42}`) and exits
// with 0 when the folder loads and every sample is colored.
import fs from 'node:fs';
import path from 'node:path';
import {execFileSync} from 'node:child_process';
import {ROOT, assert, readToml} from './util.mjs';
import {fetchPinned} from './fetch.mjs';
import {hostPlatform} from './platform.mjs';

const exe = process.platform === 'win32' ? '.exe' : '';
export const validatorLock = (root = ROOT) => path.join(root, 'tools', 'validator.lock');

/** The validator binary to use: LOOM_EXT_VALIDATE, or the one fetched into tools/bin. */
export function findValidator(root = ROOT) {
  const given = process.env.LOOM_EXT_VALIDATE;
  if (given) {
    assert(fs.existsSync(given), `LOOM_EXT_VALIDATE points at ${given}, which does not exist`);
    return path.resolve(given);
  }
  const local = path.join(root, 'tools', 'bin', `loom-ext-validate${exe}`);
  return fs.existsSync(local) ? local : null;
}

/** Download the pinned validator for this machine into tools/bin, if the lock names one. */
export async function fetchValidator({root = ROOT, offline = false} = {}) {
  const file = validatorLock(root);
  if (!fs.existsSync(file)) return null;
  const lock = readToml(file);
  assert(lock.schema === 1, 'tools/validator.lock: schema must be 1');
  const pin = lock.platforms?.[hostPlatform()];
  if (!pin) return null;
  const target = path.join(root, 'tools', 'bin', `loom-ext-validate${exe}`);
  await fetchPinned(pin.url, pin.sha256, target, {offline});
  if (process.platform !== 'win32') fs.chmodSync(target, 0o755);
  return target;
}

/** Run the validator on `directory`; returns what it printed, one object per grammar. */
export function runValidator(binary, directory, samplesFile, exec = execFileSync) {
  const args = [directory, ...(samplesFile ? ['--samples', samplesFile] : [])];
  let output;
  try { output = exec(binary, args, {encoding: 'utf8', maxBuffer: 16 * 1024 * 1024}); }
  catch (error) { throw new Error(`${path.basename(directory)}: ${(error.stderr || error.stdout || error.message).toString().trim()}`); }
  return output.split(/\r?\n/).filter(Boolean).map(line => JSON.parse(line));
}
