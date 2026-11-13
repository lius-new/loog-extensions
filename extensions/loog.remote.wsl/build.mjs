// The WASM provider is portable; the target runs a static Linux agent and needs only a POSIX shell.
import fs from 'node:fs';
import path from 'node:path';
import * as tar from 'tar';
import {buildClient, clientNotices} from '../../language-clients/rust/build.mjs';
import {buildAgents} from '../../remote/agent/build.mjs';

const here = import.meta.dirname;
const manifest = JSON.parse(fs.readFileSync(path.join(here, 'manifest.json'), 'utf8'));
if (manifest.id !== 'loog.remote.wsl') throw new Error('Unexpected WSL package identity');
const bundle = buildAgents();
const component = buildClient(here);
const dist = path.join(here, 'dist');
const folder = path.join(dist, 'package', manifest.id);
fs.rmSync(folder, {recursive: true, force: true});
fs.mkdirSync(folder, {recursive: true});
for (const file of ['manifest.json', 'README.md', 'CHANGELOG.md', 'icon.svg'])
  fs.copyFileSync(path.join(here, file), path.join(folder, file));
fs.copyFileSync(component, path.join(folder, 'extension.wasm'));
clientNotices(folder, here, {componentSource: 'remote', extraCrates: bundle.agents.map(agent =>
  ({manifest: path.join(bundle.directory, 'Cargo.toml'), platform: agent.triple}))});
fs.writeFileSync(path.join(folder, 'PROVENANCE.md'),
  '# Remote WSL\n\nProvider and embedded Agent source: https://github.com/lius-new/loog-extensions/tree/main/remote\n\n' +
  'Agent protocol: 2\n\n' + bundle.agents.map(agent => `Bundled agent (${agent.triple}): SHA-256 ${agent.sha256}\n`).join('') +
  'Agent source: remote/agent; shared text rules: the pinned Loog SDK crates/text-core.\n' +
  'musl copyright source: https://git.musl-libc.org/cgit/musl/plain/COPYRIGHT?h=v1.2.5\n\n' +
  'No WSL executable or target runtime is redistributed.\n');
const output = path.join(dist, `${manifest.id}-${manifest.version}-any.loogext`);
const names = fs.readdirSync(folder, {recursive: true, withFileTypes: true}).filter(entry => entry.isFile())
  .map(entry => path.relative(folder, path.join(entry.parentPath, entry.name)).split(path.sep).join('/')).sort();
await tar.c({file: output, cwd: path.dirname(folder), gzip: {level: 9}, portable: true, mtime: new Date(0), noDirRecurse: true},
  [manifest.id, ...names.map(name => `${manifest.id}/${name}`)]);
console.log(`Wrote ${path.relative(here, output)}`);
