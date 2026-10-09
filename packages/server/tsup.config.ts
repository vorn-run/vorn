import { readFileSync } from 'node:fs'
import { defineConfig } from 'tsup'

const { version } = JSON.parse(readFileSync('./package.json', 'utf-8'))

// The bins are plain symlinks, so the bundle needs a shebang on line 1, which only the banner can put there.
const SHEBANG = '#!/usr/bin/env node'

// Node keeps the bundle's compiled form on disk, so a warm start skips half its load.
const COMPILE_CACHE = `;try { require('module').enableCompileCache() } catch (e) {}`

const NATIVE_MODULE_PATCH = `
// Patch module resolution for Electron's utilityProcess.
//
// utilityProcess doesn't have the main process's ASAR require() patching,
// so a bare require('libsql') fails. We intercept Module._load to redirect
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
    var nativeModules = { 'libsql': true };

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
  // The `vorn` binary the installers put on PATH.
  entry: ['src/cli.ts'],
  format: ['cjs'],
  target: 'node22',
  clean: true,
  banner: {
    js: `${SHEBANG}\n${COMPILE_CACHE}\n${NATIVE_MODULE_PATCH}`
  },
  // What `vorn --version` answers. The bundle has no package.json to read.
  define: {
    __CLI_VERSION__: JSON.stringify(version)
  },
  // Bundle ALL JS dependencies so the server runs standalone in Electron's
  // utilityProcess (which cannot access modules inside the asar archive).
  //
  // libsql, a native module, remains external because it contains compiled
  // .node binaries loaded at runtime from disk.
  noExternal: [/^(?!libsql$)/],
  external: ['libsql']
})
