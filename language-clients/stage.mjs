// Prepare local acceptance packages. These are development artifacts, not signed
// releases or installations. All writes stay under language-clients/.work.
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { buildClient } from './rust/build.mjs';
import { developmentDirectory, registerDevelopment } from '../scripts/lib/development.mjs';

const repository = path.resolve(import.meta.dirname, '..');
const output = path.join(import.meta.dirname, '.work/packages');
fs.mkdirSync(output, { recursive: true });
const staged = [];
for (const id of ['loom.lsp.rust-analyzer', 'loom.lsp.gopls', 'loom.lsp.typescript-language-server']) {
  const source = path.join(repository, 'extensions', id);
  const manifest = JSON.parse(fs.readFileSync(path.join(source, 'manifest.json'), 'utf8'));
  const component = buildClient(source);
  const target = developmentDirectory(output, manifest);
  fs.mkdirSync(target, { recursive: true });
  for (const name of ['manifest.json', 'README.md', 'CHANGELOG.md', 'icon.svg']) fs.copyFileSync(path.join(source, name), path.join(target, name));
  fs.cpSync(path.join(source, 'languages'), path.join(target, 'languages'), { recursive: true });
  fs.copyFileSync(component, path.join(target, 'extension.wasm'));
  if (id.endsWith('rust-analyzer')) fs.copyFileSync(path.join(source, 'dist/package/windows-x86_64', id, 'rust-analyzer.exe'), path.join(target, 'rust-analyzer.exe'));
  if (id.endsWith('gopls')) {
    const built = path.join(source, 'dist/package/windows-x86_64', id, 'gopls.exe');
    const binary = process.env.GOPLS ?? (fs.existsSync(built) ? built : execFileSync('where.exe', ['gopls'], { encoding: 'utf8', windowsHide: true }).trim().split(/\r?\n/)[0]);
    fs.copyFileSync(binary, path.join(target, 'gopls.exe'));
  }
  if (id.endsWith('typescript-language-server')) fs.cpSync(path.join(source, 'payload/node_modules'), path.join(target, 'node_modules'), { recursive: true });
  console.log(`${id}: ${fs.statSync(path.join(target, 'extension.wasm')).size} WASM bytes`);
  staged.push({directory: target});
}
const runtimeManifest = JSON.parse(fs.readFileSync(path.join(repository, 'extensions/loom.runtime.node/manifest.json'), 'utf8'));
const runtime = developmentDirectory(output, runtimeManifest);
fs.mkdirSync(runtime, { recursive: true });
for (const name of ['manifest.json', 'README.md', 'CHANGELOG.md', 'icon.svg']) fs.copyFileSync(path.join(repository, 'extensions/loom.runtime.node', name), path.join(runtime, name));
fs.copyFileSync(process.execPath, path.join(runtime, 'node.exe'));
staged.push({directory: runtime});
registerDevelopment(output, staged);
console.log(`Local acceptance packages: ${output}`);
