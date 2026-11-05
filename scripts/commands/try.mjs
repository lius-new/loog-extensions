// try <id…> [--to dir] [--run command] [--no-run]: build the named extensions for this
// machine, unpack them into a folder Loom reads, and start Loom on that folder.
// Extensions they `requires` are built too unless the folder already holds them.
import fs from 'node:fs';
import path from 'node:path';
import {spawnSync} from 'node:child_process';
import {EXTENSIONS, WORK, assert, readJson, selectExtensions} from '../lib/util.mjs';
import {hostPlatform} from '../lib/platform.mjs';
import {parseArgs} from '../lib/cli.mjs';
import {run as unpack} from './dev.mjs';

export const usage = 'try <id…> [--source loom-dir] [--to dir] [--run command] [--no-run]    Build, unpack for Loom and start it (`cargo run` in the Loom checkout, else `loom`)';

function execute(command, args, options) {
  const result = spawnSync(command, args, {stdio: 'inherit', ...options});
  if (result.error) throw result.error;
  return result.status ?? 1;
}

/** Build one extension for `platform`; the folder its packages land in. */
function build(id, platform, env) {
  const folder = path.join(EXTENSIONS, id), dist = path.join(folder, 'dist');
  assert(fs.existsSync(path.join(folder, 'build.mjs')), `${id} has no build.mjs`);
  if (fs.existsSync(path.join(folder, 'package-lock.json')) && !fs.existsSync(path.join(folder, 'node_modules'))) {
    assert(execute('npm', ['ci'], {cwd: folder, shell: true}) === 0, `npm ci failed for ${id}`);
  }
  // Packages of an earlier version would be unpacked beside the new one.
  if (fs.existsSync(dist)) {
    for (const name of fs.readdirSync(dist)) if (name.endsWith('.loomext')) fs.rmSync(path.join(dist, name));
  }
  // Builds that cover several platforms take them as arguments; the others ignore it.
  assert(execute(process.execPath, ['build.mjs', platform], {cwd: folder, env}) === 0, `The build of ${id} failed`);
  return dist;
}

/** The ids `requires` names, without their version ranges. */
function required(id) {
  const {requires = []} = readJson(path.join(EXTENSIONS, id, 'manifest.json'));
  return requires.map(spec => String(spec).split('@')[0]);
}

/**
 * Loom reads the extensions bundled beside its executable. Nothing in a debug build
 * refreshes them, so a copy left by an older Loom shows up as an installed extension
 * at a version that no longer exists. Only build output of the checkout `cargo run` uses.
 */
export function dropStaleBundle(source) {
  const bundled = path.join(source, 'target', 'debug', 'extensions');
  if (!fs.existsSync(bundled)) return;
  fs.rmSync(bundled, {recursive: true, force: true});
  console.log(`Removed the stale bundled extensions in ${bundled}`);
}

export async function run(argv) {
  const {options, ids} = parseArgs(argv, {to: 'string', run: 'string', source: 'string', 'no-run': 'boolean'});
  assert(ids.length, 'Name the extensions to try, such as loom.theme.aura');
  // The Loom checkout: the builds compile against it and, unless told otherwise, it is what runs.
  const given = options.source || process.env.LOOM_SOURCE;
  const source = given ? path.resolve(given) : null;
  assert(!source || fs.existsSync(path.join(source, 'Cargo.toml')), `${source} is not a Loom checkout (no Cargo.toml)`);
  const env = source ? {...process.env, LOOM_SOURCE: source} : process.env;
  const destination = path.resolve(options.to || process.env.LOOM_EXTENSIONS_DIR || path.join(WORK, 'dev-extensions'));
  const everything = selectExtensions();
  const stateFile = path.join(destination, 'extensions.json');
  const installed = fs.existsSync(stateFile) ? Object.keys(readJson(stateFile).extensions ?? {}) : [];

  const wanted = [...selectExtensions(ids)];
  for (const id of wanted) {
    for (const dependency of required(id)) {
      if (everything.includes(dependency) && !wanted.includes(dependency) && !installed.includes(dependency)) {
        console.log(`${id} requires ${dependency}, building it too`);
        wanted.push(dependency);
      }
    }
  }

  const platform = hostPlatform();
  const packages = wanted.map(id => build(id, platform, env));
  await unpack([...packages, '--to', destination]);
  if (options['no-run']) {
    console.log(`Ready: LOOM_EXTENSIONS_DIR=${destination}`);
    return;
  }
  const command = options.run ?? process.env.LOOM_EXT_RUN ?? (source ? 'cargo run' : 'loom');
  console.log(`LOOM_EXTENSIONS_DIR=${destination}\n> ${command}`);
  // `cargo run` belongs in the checkout; a command of your own runs where you are.
  const cwd = source && !options.run && !process.env.LOOM_EXT_RUN ? source : undefined;
  if (cwd) dropStaleBundle(source);
  process.exitCode = execute(command, [], {shell: true, cwd, env: {...env, LOOM_EXTENSIONS_DIR: destination}});
}
