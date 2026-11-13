// The WASM provider is portable; the docker CLI runs on the client and Python in the container.
import fs from 'node:fs';
import path from 'node:path';
import {createHash} from 'node:crypto';
import * as tar from 'tar';
import {buildClient, clientNotices} from '../../language-clients/rust/build.mjs';
import {buildSearch} from '../../remote/search/build.mjs';

const here = import.meta.dirname;
const manifest = JSON.parse(fs.readFileSync(path.join(here, 'manifest.json'), 'utf8'));
if (manifest.id !== 'loog.remote.docker') throw new Error('Unexpected container package identity');
const worker = buildSearch();
const component = buildClient(here);
const dist = path.join(here, 'dist');
const folder = path.join(dist, 'package', manifest.id);
fs.rmSync(folder, {recursive: true, force: true});
fs.mkdirSync(folder, {recursive: true});
for (const file of ['manifest.json', 'README.md', 'CHANGELOG.md', 'icon.svg'])
  fs.copyFileSync(path.join(here, file), path.join(folder, file));
fs.copyFileSync(component, path.join(folder, 'extension.wasm'));
clientNotices(folder, here, {componentSource: 'remote', extraCrates: [
  {manifest:path.join(worker.directory,'Cargo.toml'),platform:'x86_64-unknown-linux-musl'}
]});
const digest = createHash('sha256').update(fs.readFileSync(path.join(here, '../../remote/search_agent.py'))).update('\n')
  .update(fs.readFileSync(path.join(here, '../../remote/agent.py'))).digest('hex');
fs.writeFileSync(path.join(folder, 'PROVENANCE.md'),
  '# Remote Containers\n\nProvider and embedded Agent source: https://github.com/lius-new/loog-extensions/tree/main/remote\n\n' +
  'Agent protocol: 1\nEmbedded Agent SHA-256: ' + digest + '\n\n' +
  'Bundled scanner target: x86_64-unknown-linux-musl\nScanner SHA-256: ' +
  createHash('sha256').update(worker.binary).digest('hex') + '\n' +
  'Scanner source: remote/search; shared text rules: the pinned Loog SDK crates/text-core.\n' +
  'musl copyright source: https://git.musl-libc.org/cgit/musl/plain/COPYRIGHT?h=v1.2.5\n\n' +
  'No Docker executable or target Python runtime is redistributed.\n');
const output = path.join(dist, `${manifest.id}-${manifest.version}-any.loogext`);
const names = fs.readdirSync(folder, {recursive: true, withFileTypes: true}).filter(entry => entry.isFile())
  .map(entry => path.relative(folder, path.join(entry.parentPath, entry.name)).split(path.sep).join('/')).sort();
await tar.c({file: output, cwd: path.dirname(folder), gzip: {level: 9}, portable: true, mtime: new Date(0), noDirRecurse: true},
  [manifest.id, ...names.map(name => `${manifest.id}/${name}`)]);
console.log(`Wrote ${path.relative(here, output)}`);
