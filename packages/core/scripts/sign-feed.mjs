#!/usr/bin/env node
// Signs a release's update feed with the release key, so the app trusts it:
//
//   vorn-update.json.sig   base64 Ed25519 signature over vorn-update.json's exact bytes
//
//   --dir <dir>   where vorn-update.json is, and where the signature goes (default: dist-native)
//   --public      prints the key's public half, base64, for crates/vorn-app/update-key.pub
//
// The key is VORN_UPDATE_SIGNING_KEY: an Ed25519 private key as PKCS#8 PEM. It
// is never written anywhere by this script.
import { createPrivateKey, createPublicKey, sign, verify } from 'node:crypto'
import { readFileSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

export const FEED = 'vorn-update.json'
export const SIGNATURE = `${FEED}.sig`

/** The raw 32-byte public key of an Ed25519 key object, base64. */
export function publicBase64(key) {
  const der = createPublicKey(key).export({ format: 'der', type: 'spki' })
  // An Ed25519 SubjectPublicKeyInfo is a fixed 12-byte header and the 32-byte key.
  return der.subarray(der.length - 32).toString('base64')
}

/** `body` signed with `key`, base64. */
export function signFeed(body, key) {
  return sign(null, body, key).toString('base64')
}

/** Whether `signature` (base64) is `publicKey`'s (raw, base64) over `body`. */
export function verifyFeed(body, signature, publicKey) {
  const spki = Buffer.concat([
    Buffer.from('302a300506032b6570032100', 'hex'),
    Buffer.from(publicKey, 'base64')
  ])
  const key = createPublicKey({ key: spki, format: 'der', type: 'spki' })
  return verify(null, body, key, Buffer.from(signature, 'base64'))
}

function main() {
  const argv = process.argv.slice(2)
  const pem = process.env.VORN_UPDATE_SIGNING_KEY
  if (!pem) throw new Error('VORN_UPDATE_SIGNING_KEY is not set')
  const key = createPrivateKey(pem)
  if (key.asymmetricKeyType !== 'ed25519') throw new Error('the signing key is not Ed25519')
  if (argv.includes('--public')) {
    console.log(publicBase64(key))
    return
  }
  const at = argv.indexOf('--dir')
  const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..', '..')
  const dir = path.resolve(at >= 0 ? argv[at + 1] : path.join(repo, 'dist-native'))
  const body = readFileSync(path.join(dir, FEED))
  const signature = signFeed(body, key)
  // Checked against the key the app embeds, so a release never ships a feed its apps refuse.
  const embedded = readFileSync(
    path.join(repo, 'packages/core/crates/vorn-app/update-key.pub'),
    'utf8'
  ).trim()
  if (embedded !== publicBase64(key)) {
    throw new Error('the signing key is not the one crates/vorn-app/update-key.pub names')
  }
  if (!verifyFeed(body, signature, embedded)) throw new Error('the signature does not verify')
  writeFileSync(path.join(dir, SIGNATURE), `${signature}\n`)
  console.log(`wrote ${SIGNATURE}`)
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    main()
  } catch (e) {
    console.error(`sign-feed: ${e.message}`)
    process.exit(1)
  }
}
