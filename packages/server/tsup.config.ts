import { readFileSync } from 'node:fs'
import { defineConfig } from 'tsup'

const { version } = JSON.parse(readFileSync('./package.json', 'utf-8'))

// Prepended to both entries. `cli.cjs` is what both bins point at -- `vorn` and
// the `vorn-server` alias -- and Yarn links a bin as a plain symlink: without
// this the shell runs it as sh and it dies partway through the bundle with a
// syntax error. It has to live in the
// banner rather than at the top of `src/cli.ts`, because the banner is emitted
// first and a shebang is only honoured on line 1. Node ignores it in
// `index.cjs`, which is required rather than executed.
const SHEBANG = '#!/usr/bin/env node'

const NATIVE_MODULE_PATCH = `
;try { require('module').enableCompileCache() } catch (e) {}
// Patch module resolution for Electron's utilityProcess.
//
// utilityProcess doesn't have the main process's ASAR require() patching,
// so bare require('node-pty') and require('libsql') fail. We intercept Module._load to redirect
// native module names to their absolute paths in app.asar.unpacked/node_modules/.
//
// The parent process passes VORN_NATIVE_MODULES_PATH as an env var
// pointing to the unpacked node_modules directory.
//
// @see https://electron-vite.org/guide/assets
// @see https://github.com/electron/electron/issues/8727
;(function() {
  var nativePath = process.env.VORN_NATIVE_MODULES_PATH;
  if (!nativePath) return;
  try {
    var Module = require('module');
    var path = require('path');
    var nativeModules = { 'node-pty': true, 'libsql': true };

    var origLoad = Module._load;
    Module._load = function(request, parent, isMain) {
      if (nativeModules[request]) {
        return origLoad.call(this, path.join(nativePath, request), parent, isMain);
      }
      return origLoad.call(this, request, parent, isMain);
    };
  } catch(e) { console.error('[native-module-patch] failed:', e); }
})();
`

export default defineConfig({
  // Two entries: `index` is what Electron's utilityProcess spawns, `cli` is the
  // `vorn` binary the installers put on PATH. They are bundled independently
  // rather than code-split, because each runs as its own process and a shared
  // chunk would only add a require() hop.
  entry: ['src/index.ts', 'src/cli.ts'],
  format: ['cjs'],
  target: 'node22',
  clean: true,
  banner: {
    js: `${SHEBANG}\n${NATIVE_MODULE_PATCH}`
  },
  // What `vorn --version` answers. The bundle has no package.json to read.
  define: {
    __CLI_VERSION__: JSON.stringify(version)
  },
  // Bundle ALL JS dependencies so the server runs standalone in Electron's
  // utilityProcess (which cannot access modules inside the asar archive).
  //
  // Native modules (node-pty, libsql) remain external because they
  // contain compiled .node binaries loaded at runtime from disk.
  noExternal: [/^(?!node-pty$|libsql$)/],
  external: ['node-pty', 'libsql']
})
