// Small shared helpers: paths that cannot escape, hashes, atomic writes, TOML and
// the directories the tools work in.
import fs from 'node:fs';
import path from 'node:path';
import {createHash, randomUUID} from 'node:crypto';
import {execFileSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import {parse, stringify} from 'smol-toml';

// LOOG_EXT_ROOT points the tools at another checkout (the tests use a temporary one).
export const ROOT = path.resolve(process.env.LOOG_EXT_ROOT ?? fileURLToPath(new URL('../../', import.meta.url)));
export const EXTENSIONS = path.join(ROOT, 'extensions');
export const DIST = path.join(ROOT, 'dist');
export const WORK = path.join(ROOT, '.work');

export const HASH = /^[a-f0-9]{64}$/;
export const assert = (condition, message) => { if (!condition) throw new Error(message); };

export const readJson = file => JSON.parse(fs.readFileSync(file, 'utf8'));
export const readToml = file => parse(fs.readFileSync(file, 'utf8'));
export const toToml = data => stringify(data);

export const hash = value => createHash('sha256').update(value).digest('hex');
export const fileHash = file => hash(fs.readFileSync(file));

/** The value with every object's keys sorted, so equal data hashes equally. */
export function canonical(value) {
  if (Array.isArray(value)) return value.map(canonical);
  if (value && typeof value === 'object') {
    return Object.fromEntries(Object.keys(value).sort().map(key => [key, canonical(value[key])]));
  }
  return value;
}

/** A path inside a package: relative, with `/`, never climbing out. */
export function relative(value, label = 'path') {
  assert(typeof value === 'string' && value && !value.startsWith('/') && !/[\\:\0]/.test(value)
    && !value.split('/').some(part => part === '..' || part === '.' || part === ''),
  `Invalid ${label}: ${value}`);
  return value;
}

/** `value` resolved under `root`, after links, and required to stay there. */
export function inside(root, value) {
  const base = fs.realpathSync(root);
  if (value === '.') return base;
  const target = path.resolve(root, relative(value));
  const actual = fs.realpathSync(target);
  assert(actual === base || actual.startsWith(base + path.sep), `Path escapes its folder: ${value}`);
  return actual;
}

/** Every file under `root` as sorted `/` paths. Anything but files and folders is an error. */
export function files(root) {
  const result = [];
  const walk = directory => {
    const entries = fs.readdirSync(directory, {withFileTypes: true})
      .sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0));
    for (const entry of entries) {
      const full = path.join(directory, entry.name);
      if (entry.isDirectory()) walk(full);
      else {
        assert(entry.isFile(), `Only files and folders are allowed: ${full}`);
        result.push(path.relative(root, full).split(path.sep).join('/'));
      }
    }
  };
  walk(root);
  return result.sort();
}

export function copyTree(source, target) {
  for (const file of files(source)) {
    const output = path.join(target, ...file.split('/'));
    fs.mkdirSync(path.dirname(output), {recursive: true});
    fs.copyFileSync(path.join(source, ...file.split('/')), output);
  }
  fs.mkdirSync(target, {recursive: true});
}

export function writeAtomic(file, value) {
  fs.mkdirSync(path.dirname(file), {recursive: true});
  const temporary = `${file}.${randomUUID()}.tmp`;
  try {
    fs.writeFileSync(temporary, value);
    fs.renameSync(temporary, file);
  } finally {
    fs.rmSync(temporary, {force: true});
  }
}

/** Remove `target`, but only when it lies strictly inside `base`. */
export function removeUnder(base, target) {
  const root = path.resolve(base), absolute = path.resolve(target);
  assert(absolute.startsWith(root + path.sep), `Refusing to remove outside ${root}: ${absolute}`);
  fs.rmSync(absolute, {recursive: true, force: true});
}

export function run(command, args, options = {}) {
  return execFileSync(command, args, {
    cwd: ROOT, encoding: 'utf8', timeout: 600000, maxBuffer: 32 * 1024 * 1024, ...options,
  });
}

/** The extension folders to work on: the named ids, or all of them. */
export function selectExtensions(ids, root = EXTENSIONS) {
  const all = fs.existsSync(root)
    ? fs.readdirSync(root, {withFileTypes: true}).filter(entry => entry.isDirectory()).map(entry => entry.name).sort()
    : [];
  if (!ids?.length) return all;
  for (const id of ids) assert(all.includes(id), `Unknown extension: ${id}`);
  assert(new Set(ids).size === ids.length, 'Name each extension once');
  return [...ids].sort();
}
