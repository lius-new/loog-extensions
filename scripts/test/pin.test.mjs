import test from 'node:test';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {digestOf, pinPack} from '../lib/pin.mjs';

const sha = text => createHash('sha256').update(text).digest('hex');

/** A fetch that serves a fixed table and records what it was asked. */
function server(table) {
  const calls = [];
  const fetcher = async (url, options = {}) => {
    calls.push(`${options.method ?? 'GET'} ${url}`);
    const entry = table[url];
    if (!entry) return {ok: false, status: 404, body: null, headers: new Headers()};
    if (typeof entry === 'object' && !(entry instanceof Uint8Array) && !entry.text) return {ok: true, status: 200, json: async () => entry, body: [], headers: new Headers()};
    if (entry.text) return {ok: true, status: 200, text: async () => entry.text, body: [], headers: new Headers({'content-length': String(entry.length ?? 0)})};
    return {ok: true, status: 200, body: [Buffer.from(entry)], headers: new Headers()};
  };
  return {fetcher, calls};
}

const github = {
  kind: 'archive', version: '1.0.0', upstream: {source: 'github', repository: 'acme/ls', tag: 'v1'},
  licenses: [{name: 'LICENSE', url: 'https://example.com/LICENSE'}],
  platforms: {'linux-x86_64': {asset: 'ls.tar.gz', format: 'tar.gz', executables: ['ls']}},
};

test('a digest and size are measured from the download', async () => {
  const { fetcher } = server({'https://x.test/a': 'hello'});
  assert.deepEqual(await digestOf('https://x.test/a', fetcher), {sha256: sha('hello'), size: 5});
  await assert.rejects(digestOf('https://x.test/missing', fetcher), /Download failed \(404\)/);
});

test('a publisher checksum is used when the release gives one, and the download is hashed when not', async () => {
  const url = 'https://github.com/acme/ls/releases/download/v1/ls.tar.gz';
  const api = 'https://api.github.com/repos/acme/ls/releases/tags/v1';
  const { fetcher, calls } = server({
    [api]: {assets: [{name: 'ls.tar.gz', size: 12, digest: `sha256:${'a'.repeat(64)}`}]},
    'https://example.com/LICENSE': 'MIT text',
  });
  const pins = await pinPack('loom.lsp.ls', {pack: github}, {}, {fetcher, log() {}});
  assert.deepEqual(pins.platforms['linux-x86_64'], {url, sha256: 'a'.repeat(64), size: 12});
  assert.deepEqual(pins.licenses.LICENSE, {url: 'https://example.com/LICENSE', sha256: sha('MIT text'), size: 8});
  assert.ok(!calls.includes(`GET ${url}`), 'the archive itself was not downloaded');

  const measured = server({
    [api]: {assets: [{name: 'ls.tar.gz', size: 5}]},
    [url]: 'bytes', 'https://example.com/LICENSE': 'MIT text',
  });
  const second = await pinPack('loom.lsp.ls', {pack: github}, {}, {fetcher: measured.fetcher, log() {}});
  assert.equal(second.platforms['linux-x86_64'].sha256, sha('bytes'));
  const wrong = server({[api]: {assets: [{name: 'ls.tar.gz', size: 99}]}, [url]: 'bytes', 'https://example.com/LICENSE': 'x'});
  await assert.rejects(pinPack('loom.lsp.ls', {pack: github}, {}, {fetcher: wrong.fetcher, log() {}}), /Downloaded size differs/);
});

test('old pins are kept unless the URL changed or a refresh is asked for', async () => {
  const url = 'https://github.com/acme/ls/releases/download/v1/ls.tar.gz';
  const kept = {schema: 1, platforms: {'linux-x86_64': {url, sha256: 'b'.repeat(64), size: 1}}, licenses: {LICENSE: {url: 'https://example.com/LICENSE', sha256: 'c'.repeat(64), size: 1}}};
  const { fetcher, calls } = server({});
  const pins = await pinPack('loom.lsp.ls', {pack: github}, kept, {fetcher, log() {}});
  assert.deepEqual(pins.platforms, kept.platforms);
  assert.deepEqual(calls, [], 'nothing was fetched');
  await assert.rejects(pinPack('loom.lsp.ls', {pack: github}, kept, {fetcher, refresh: true, log() {}}), /GitHub release not found/);
});

test('node archives are pinned from the published checksum list', async () => {
  const pack = {kind: 'archive', version: '22.1.0', upstream: {source: 'nodejs'}, platforms: {'linux-x86_64': {asset: 'node-v22.1.0-linux-x64.tar.xz', format: 'tar.xz', executables: ['node']}}};
  const { fetcher } = server({'https://nodejs.org/dist/v22.1.0/SHASUMS256.txt': {text: `${'d'.repeat(64)}  node-v22.1.0-linux-x64.tar.xz\n`, length: 31}});
  const pins = await pinPack('loom.runtime.node', {pack}, {}, {fetcher, log() {}});
  assert.equal(pins.platforms['linux-x86_64'].sha256, 'd'.repeat(64));
});
