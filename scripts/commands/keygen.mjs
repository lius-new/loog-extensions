// keygen --out file: make an index signing key. The private key goes to the file
// (keep it secret; CI reads it from a secret); the public key is printed for Loom.
import fs from 'node:fs';
import path from 'node:path';
import {assert} from '../lib/util.mjs';
import {generateKeyPair} from '../lib/sign.mjs';
import {parseArgs} from '../lib/cli.mjs';

export const usage = 'keygen --out file    Make an index signing key (prints the public key to give Loom)';

export async function run(argv) {
  const {options} = parseArgs(argv, {out: 'string'});
  assert(options.out, 'Say where to write the private key: --out <file>');
  const target = path.resolve(options.out);
  assert(!fs.existsSync(target), `${target} already exists; it is not overwritten`);
  const { privatePem, publicRaw } = generateKeyPair();
  fs.mkdirSync(path.dirname(target), {recursive: true});
  fs.writeFileSync(target, privatePem, {mode: 0o600});
  console.log(`Private key written to ${target}. Keep it secret.`);
  console.log(`Public key (raw Ed25519, base64): ${publicRaw}`);
}
