// Runs the real command line against a temporary checkout.
import fs from 'node:fs';
import path from 'node:path';
import {spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import {language, server, theme, workspace, write} from './fixtures.mjs';

const CLI = fileURLToPath(new URL('../loom-ext.mjs', import.meta.url));

export function checkout(t, {bundled = ['loom.theme.demo']} = {}) {
  const root = workspace(t);
  write(root, 'loom-ext.toml', `schema = 1
repository = "acme/extensions"
bundled = ${JSON.stringify(bundled)}

[index]
key-id = "test-1"
validity-days = 30
`);
  fs.mkdirSync(path.join(root, 'extensions'), {recursive: true});
  return {root, extensions: path.join(root, 'extensions')};
}

export function populate({extensions}, what = ['theme', 'language']) {
  if (what.includes('theme')) theme(extensions);
  if (what.includes('language')) language(extensions);
  if (what.includes('server')) server(extensions);
}

export function cli(root, args, env = {}) {
  const result = spawnSync(process.execPath, [CLI, ...args], {
    cwd: root, encoding: 'utf8', env: {...process.env, LOOM_EXT_ROOT: root, ...env},
  });
  return {status: result.status, out: result.stdout, err: result.stderr, all: result.stdout + result.stderr};
}

export {write};
