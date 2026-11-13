// Target artifacts are owned and bundled by the SSH plugin, never by Loog.
import fs from 'node:fs';
import path from 'node:path';
import {execFileSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {gzipSync} from 'node:zlib';
import {prepareSdk} from '../../scripts/lib/sdk.mjs';

export function buildSearch() {
  prepareSdk(path.resolve(import.meta.dirname, '../..'), process.env.LOOG_SOURCE);
  const target = 'x86_64-unknown-linux-musl';
  const env = {...process.env, CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER: 'rust-lld'};
  const directory = import.meta.dirname;
  execFileSync('cargo', ['build', '--manifest-path', path.join(directory, 'Cargo.toml'),
    '--locked', '--release', '--target', target], {cwd: directory, env, stdio: 'inherit', windowsHide: true});
  const targetDir = process.env.CARGO_TARGET_DIR ? path.resolve(directory, process.env.CARGO_TARGET_DIR) : path.join(directory, 'target');
  const binary = fs.readFileSync(path.join(targetDir, target, 'release/loog-ssh-search'));
  const compressed = gzipSync(binary, {level: 9});
  if (binary.length > 32 * 1024 * 1024 || compressed.length > 8 * 1024 * 1024)
    throw new Error('Bundled search worker exceeds its deployment budget');
  const destination = path.join(directory, 'target');
  fs.mkdirSync(destination, {recursive: true});
  fs.writeFileSync(path.join(destination, 'bundle.gz'), compressed);
  fs.writeFileSync(path.join(destination, 'bundle.txt'), `${binary.length}\n${createHash('sha256').update(binary).digest('hex')}\n`);
  return {binary, compressed, directory};
}
