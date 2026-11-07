import fs from 'node:fs';
import path from 'node:path';
import {createHash} from 'node:crypto';
import {execFileSync} from 'node:child_process';
import {buildClient, clientNotices} from '../../rust/build.mjs';

const directory = import.meta.dirname;
const source = path.resolve(directory, '../../..');
process.env.LOOG_SOURCE ??= path.resolve(directory, '../../../../..');
const loog = path.resolve(process.env.LOOG_SOURCE);
const output = buildClient(directory);
clientNotices(path.dirname(output), directory, {source});

const digest = bytes => createHash('sha256').update(bytes).digest('hex');
const files = (root, name) => fs.statSync(path.join(root, name)).isDirectory()
  ? fs.readdirSync(path.join(root, name)).sort().flatMap(child => files(root, name + '/' + child))
  : [name];
const inputs = [];
for (const name of ['language-clients/fixtures/native-ui/build.mjs',
  'language-clients/fixtures/native-ui/rust/Cargo.toml', 'language-clients/fixtures/native-ui/rust/Cargo.lock',
  'language-clients/fixtures/native-ui/rust/build.rs', 'language-clients/fixtures/native-ui/rust/src',
  'language-clients/rust/build.mjs', 'language-clients/rust/src',
  'language-clients/rust/componentize/Cargo.toml', 'language-clients/rust/componentize/Cargo.lock',
  'language-clients/rust/componentize/src']) {
  for (const file of files(source, name)) inputs.push(['extensions/' + file, digest(fs.readFileSync(path.join(source, file)))]);
}
for (const file of files(loog, 'wit/loog-extension')) inputs.push(['loog/' + file, digest(fs.readFileSync(path.join(loog, file)))]);
inputs.sort(([a], [b]) => a.localeCompare(b, 'en'));
fs.writeFileSync(path.join(path.dirname(output), 'build.json'), JSON.stringify({
  sourceFingerprint: digest(JSON.stringify(inputs)), wasmSha256: digest(fs.readFileSync(output)),
  rustc: execFileSync('rustc', ['--version'], {encoding: 'utf8', windowsHide: true}).trim(), inputs,
}, null, 2) + '\n');
