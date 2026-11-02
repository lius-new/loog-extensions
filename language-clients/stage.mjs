// Prepare local acceptance packages. These are development artifacts, not signed
// releases or installations. All writes stay under language-clients/.work.
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { buildClient } from './build.mjs';

const repository = path.resolve(import.meta.dirname, '..');
const output = path.join(import.meta.dirname, '.work/packages');
fs.mkdirSync(output, { recursive: true });
for (const id of ['loom.lsp.rust-analyzer', 'loom.lsp.gopls', 'loom.lsp.typescript-language-server']) {
  const source = path.join(repository, 'extensions', id);
  const target = path.join(output, id);
  fs.mkdirSync(target, { recursive: true });
  for (const name of ['manifest.json', 'README.md', 'CHANGELOG.md', 'icon.svg']) fs.copyFileSync(path.join(source, name), path.join(target, name));
  fs.cpSync(path.join(source, 'languages'), path.join(target, 'languages'), { recursive: true });
  fs.copyFileSync(buildClient(source), path.join(target, 'extension.wasm'));
  if (id.endsWith('rust-analyzer')) fs.copyFileSync(path.join(source, 'dist/package/windows-x86_64', id, 'rust-analyzer.exe'), path.join(target, 'rust-analyzer.exe'));
  if (id.endsWith('gopls')) {
    const built = path.join(source, 'dist/package/windows-x86_64', id, 'gopls.exe');
    const binary = process.env.GOPLS ?? (fs.existsSync(built) ? built : execFileSync('where.exe', ['gopls'], { encoding: 'utf8', windowsHide: true }).trim().split(/\r?\n/)[0]);
    fs.copyFileSync(binary, path.join(target, 'gopls.exe'));
  }
  if (id.endsWith('typescript-language-server')) fs.cpSync(path.join(source, 'payload/node_modules'), path.join(target, 'node_modules'), { recursive: true });
  console.log(`${id}: ${fs.statSync(path.join(target, 'extension.wasm')).size} WASM bytes`);
}
const runtime = path.join(output, 'loom.runtime.node');
fs.mkdirSync(runtime, { recursive: true });
for (const name of ['manifest.json', 'README.md', 'CHANGELOG.md', 'icon.svg']) fs.copyFileSync(path.join(repository, 'extensions/loom.runtime.node', name), path.join(runtime, name));
fs.copyFileSync(process.execPath, path.join(runtime, 'node.exe'));
console.log(`Local acceptance packages: ${output}`);
