# Signed update feed

Every release publishes `vorn-update.json`, the feed installed apps update from,
and `vorn-update.json.sig`, an Ed25519 signature over the feed's exact bytes.

- The app trusts a feed only when its signature checks out against the public
  key built into it (`packages/core/crates/vorn-app/update-key.pub`, 32 bytes,
  base64). Only then does it read the SHA-256 and size of the download from it.
  There is no unsigned path: a build with no key, or a feed with no valid
  signature, does not update and says so.
- `install.sh` and `install.ps1` fetch the same feed and check the downloaded
  file against its `checksums` entry. A file that does not match is removed and
  not installed.

## Release key

The private key never enters the repository. It lives in one place, the
`VORN_UPDATE_SIGNING_KEY` Actions secret, as an Ed25519 private key in PKCS#8
PEM.

Set it up once:

1. Make the key on a trusted machine, outside any checkout:

   ```sh
   node -e "const {generateKeyPairSync}=require('crypto');const {privateKey}=generateKeyPairSync('ed25519');process.stdout.write(privateKey.export({type:'pkcs8',format:'pem'}))" > vorn-update.pem
   ```

2. Print its public half and commit it as the key the app checks against:

   ```sh
   VORN_UPDATE_SIGNING_KEY="$(cat vorn-update.pem)" node packages/core/scripts/sign-feed.mjs --public \
     > packages/core/crates/vorn-app/update-key.pub
   ```

3. Add the PEM as the repository secret `VORN_UPDATE_SIGNING_KEY`
   (Settings › Secrets and variables › Actions), then store the file offline
   and delete the local copy.

From then on the release workflow signs each feed after writing it
(`sign-feed.mjs`, in `native-app.yml`), checks the signature against
`update-key.pub`, and uploads both files. A tagged release without the secret
fails before its feed goes up.

## Rehearsals

`update-path.mjs` serves a feed of its own to an installed app. It signs that
feed with a key it makes for the run and names the public half in
`VORN_UPDATE_KEY` beside `VORN_UPDATE_FEED`. The app reads `VORN_UPDATE_KEY`
only when `VORN_UPDATE_FEED` is set too.
