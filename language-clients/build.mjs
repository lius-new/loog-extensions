// Development build adapter. Published artifacts contain their own component;
// Loom's SDK is a build dependency, never a language client dependency at runtime.
import path from 'node:path';
import fs from 'node:fs';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';

export function buildClient(directory, { checkOnly = false } = {}) {
  const loom = path.resolve(process.env.LOOM_SOURCE ?? path.join(import.meta.dirname, '../../..'));
  const sdk = path.join(loom, 'sdk/typescript');
  const tools = path.join(loom, 'tests/extensions/js-guest/node_modules');
  const work = path.join(directory, '.work/client');
  fs.mkdirSync(work, { recursive: true });
  const configuration = path.join(work, 'tsconfig.json');
  fs.writeFileSync(configuration, JSON.stringify({
    extends: path.join(sdk, 'tsconfig.json'),
    include: [path.join(directory, 'src/**/*.ts'), path.join(import.meta.dirname, '*.ts'), path.join(sdk, 'src/**/*.ts'), path.join(sdk, 'generated/**/*.d.ts')].map(value => value.replaceAll('\\', '/')),
    exclude: [path.join(import.meta.dirname, '*.test.ts').replaceAll('\\', '/')],
  }, null, 2));
  execFileSync(process.execPath, [path.join(tools, 'typescript/bin/tsc'), '-p', configuration], { stdio: 'inherit' });
  if (checkOnly) return;
  const output = path.join(work, 'extension.wasm');
  const hash = createHash('sha256');
  const add = directory => {
    for (const name of fs.readdirSync(directory).sort()) {
      const file = path.join(directory, name);
      if (fs.statSync(file).isDirectory()) add(file);
      else { hash.update(name); hash.update(fs.readFileSync(file)); }
    }
  };
  add(path.join(directory, 'src')); add(path.join(sdk, 'src')); add(path.join(sdk, 'generated')); add(path.join(loom, 'wit/loom-extension'));
  for (const name of ['client.ts', 'protocol.ts', 'results.ts', 'build.mjs']) hash.update(fs.readFileSync(path.join(import.meta.dirname, name)));
  for (const file of [path.join(sdk, 'build.mjs'), path.join(sdk, 'tsconfig.json'), path.join(loom, 'tests/extensions/js-guest/package-lock.json')]) hash.update(fs.readFileSync(file));
  const key = hash.digest('hex');
  const stamp = path.join(work, 'source.sha256');
  if (fs.existsSync(output) && fs.existsSync(stamp) && fs.readFileSync(stamp, 'utf8') === key) return output;
  execFileSync(process.execPath, [path.join(sdk, 'build.mjs'), path.join(directory, 'src/main.ts'), output], { stdio: 'inherit' });
  if (fs.statSync(output).size > 16 * 1024 * 1024) throw new Error('language component exceeds the 16 MiB P0 budget');
  fs.writeFileSync(stamp, key);
  return output;
}
