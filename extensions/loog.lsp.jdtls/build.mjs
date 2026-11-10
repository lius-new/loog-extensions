// Packages the pinned Eclipse JDT LS distribution and the Loog WASM client.
import fs from 'node:fs';
import path from 'node:path';
import {createHash} from 'node:crypto';
import * as tar from 'tar';
import {buildClient, clientNotices} from '../../language-clients/rust/build.mjs';

const here = import.meta.dirname;
const pin = JSON.parse(fs.readFileSync(path.join(here, 'pins.json'), 'utf8'));
const manifest = JSON.parse(fs.readFileSync(path.join(here, 'manifest.json'), 'utf8'));
const component = buildClient(here);
const dist = path.join(here, 'dist');
const packageRoot = path.join(dist, 'package');
const folder = path.join(packageRoot, manifest.id);
const server = path.join(folder, 'server');
const downloads = path.join(here, '.work', 'downloads');
const hash = bytes => createHash('sha256').update(bytes).digest('hex');

async function download({url, sha256}) {
  const destination = path.join(downloads, sha256);
  if (fs.existsSync(destination) && hash(fs.readFileSync(destination)) === sha256) return destination;
  if (new URL(url).protocol !== 'https:') throw new Error(`Only HTTPS downloads are allowed: ${url}`);
  const response = await fetch(url, {signal: AbortSignal.timeout(1800000)});
  if (!response.ok) throw new Error(`${url}: HTTP ${response.status}`);
  const bytes = Buffer.from(await response.arrayBuffer());
  if (hash(bytes) !== sha256) throw new Error(`${url}: SHA-256 mismatch`);
  fs.mkdirSync(downloads, {recursive: true});
  fs.writeFileSync(destination + '.partial', bytes);
  fs.renameSync(destination + '.partial', destination);
  return destination;
}

const archive = await download(pin);
fs.rmSync(packageRoot, {recursive: true, force: true});
fs.mkdirSync(server, {recursive: true});
await tar.x({file: archive, cwd: server, strict: true, filter: (_, entry) => (
  entry.type === 'File' || entry.type === 'Directory'
)});
if (!fs.statSync(path.join(server, pin.launcher)).isFile()) throw new Error('Missing Equinox launcher');
for (const name of ['config_linux', 'config_linux_arm', 'config_mac', 'config_mac_arm', 'config_win']) {
  if (!fs.statSync(path.join(server, name, 'config.ini')).isFile()) throw new Error(`Missing ${name}/config.ini`);
}
for (const file of ['manifest.json', 'README.md', 'CHANGELOG.md', 'icon.svg', 'launch.mjs']) {
  fs.copyFileSync(path.join(here, file), path.join(folder, file));
}
fs.cpSync(path.join(here, 'languages'), path.join(folder, 'languages'), {recursive: true});
fs.copyFileSync(component, path.join(folder, 'extension.wasm'));
clientNotices(folder, here);
for (const [name, license] of Object.entries(pin.licenses)) {
  fs.copyFileSync(await download(license), path.join(folder, 'licenses', name));
}
fs.writeFileSync(path.join(folder, 'PROVENANCE.md'),
  `# Eclipse JDT LS ${pin.version}\n\nUpstream: ${pin.url}\nSHA-256: ${pin.sha256}\n`);
const names = fs.readdirSync(folder, {recursive: true, withFileTypes: true}).filter(entry => entry.isFile())
  .map(entry => path.relative(folder, path.join(entry.parentPath, entry.name)).split(path.sep).join('/')).sort();
const output = path.join(dist, `${manifest.id}-${manifest.version}-any.loogext`);
await tar.c({file: output, cwd: packageRoot, gzip: {level: 9}, portable: true, mtime: new Date(0), noDirRecurse: true},
  [manifest.id, ...names.map(name => `${manifest.id}/${name}`)]);
console.log(`Wrote ${path.relative(here, output)}`);
