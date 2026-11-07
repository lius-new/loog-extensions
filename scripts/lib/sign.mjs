// Ed25519 signatures over the exact bytes of index.json. The private key lives only in
// CI (or on the maintainer's machine); Loog carries the public keys, each with a
// key id, so a key can be replaced without breaking installed copies.
import crypto from 'node:crypto';
import {assert} from './util.mjs';

// DER prefix of an Ed25519 public key in SubjectPublicKeyInfo form; the raw key follows.
const SPKI_PREFIX = Buffer.from('302a300506032b6570032100', 'hex');

export function generateKeyPair() {
  const {publicKey, privateKey} = crypto.generateKeyPairSync('ed25519');
  return {
    privatePem: privateKey.export({type: 'pkcs8', format: 'pem'}),
    publicRaw: publicKey.export({type: 'spki', format: 'der'}).subarray(SPKI_PREFIX.length).toString('base64'),
  };
}

/** The raw 32-byte public key (base64) belonging to a private key in PEM form. */
export function publicKeyOf(privatePem) {
  const publicKey = crypto.createPublicKey(crypto.createPrivateKey(privatePem));
  return publicKey.export({type: 'spki', format: 'der'}).subarray(SPKI_PREFIX.length).toString('base64');
}

const publicFromRaw = raw => {
  const bytes = Buffer.from(raw, 'base64');
  assert(bytes.length === 32, 'An Ed25519 public key is 32 bytes');
  return crypto.createPublicKey({key: Buffer.concat([SPKI_PREFIX, bytes]), format: 'der', type: 'spki'});
};

/** The signature file for `bytes`: which key signed, and the signature. */
export function signIndex(bytes, privatePem, keyId) {
  assert(/^[\w.-]+$/.test(keyId), 'Invalid key id');
  const signature = crypto.sign(null, bytes, crypto.createPrivateKey(privatePem)).toString('base64');
  return `${JSON.stringify({keyId, algorithm: 'ed25519', signature}, null, 2)}\n`;
}

/** Whether `signatureFile` is a good signature of `bytes` by one of `keys` ({keyId: rawBase64}). */
export function verifyIndex(bytes, signatureFile, keys) {
  let envelope;
  try { envelope = JSON.parse(signatureFile); } catch { return {ok: false, reason: 'the signature file is not JSON'}; }
  if (envelope.algorithm !== 'ed25519') return {ok: false, reason: 'unsupported signature algorithm'};
  const raw = keys[envelope.keyId];
  if (!raw) return {ok: false, reason: `unknown key ${envelope.keyId}`};
  let good = false;
  try { good = crypto.verify(null, bytes, publicFromRaw(raw), Buffer.from(envelope.signature, 'base64')); } catch { /* reported below */ }
  return good ? {ok: true, keyId: envelope.keyId} : {ok: false, reason: 'the signature does not match the index'};
}
