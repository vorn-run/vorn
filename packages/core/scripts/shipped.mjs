#!/usr/bin/env node
// Builds the shipped set: the packs shipped.toml pins, from a checkout of the
// extensions repository at the lock's rev, into packages/core/shipped/, where
// vornd's build embeds them. Each pack is checked against its pinned sha256.
//
//   node scripts/shipped.mjs --from <extensions checkout>
//   node scripts/shipped.mjs          clones the repository at the rev into target/
//
// Until the shipped extensions are released, this is how CI and packaging get
// them; the packs are byte-for-byte the ones the extensions' release builds.
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { copyFileSync, existsSync, mkdirSync, readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const core = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const REPO = 'https://github.com/vorn-run/extensions.git';

/** The lock: its rev and each extension's id, path, version and sha256. */
export function readLock(text) {
  const rev = text.match(/^rev\s*=\s*"([^"]+)"/m)?.[1];
  const extensions = text
    .split(/^\[\[extension\]\]\s*$/m)
    .slice(1)
    .map((block) => {
      const field = (key) => block.match(new RegExp(`^${key}\\s*=\\s*"([^"]*)"`, 'm'))?.[1];
      return { id: field('id'), path: field('path'), version: field('version'), sha256: field('sha256') };
    });
  return { rev, extensions };
}

function run(cmd, args, cwd) {
  execFileSync(cmd, args, { cwd, stdio: 'inherit' });
}

function main() {
  const args = process.argv.slice(2);
  const at = args.indexOf('--from');
  const lock = readLock(readFileSync(join(core, 'shipped.toml'), 'utf8'));
  if (!lock.rev || lock.extensions.some((e) => !e.id || !e.version || !e.sha256)) {
    throw new Error('shipped.toml: a rev, and an id, version and sha256 for every extension');
  }
  let checkout;
  if (at >= 0 && args[at + 1]) {
    checkout = resolve(args[at + 1]);
  } else {
    checkout = join(core, 'target', 'extensions-src');
    if (!existsSync(join(checkout, '.git'))) {
      run('git', ['clone', '--quiet', REPO, checkout], core);
    }
    run('git', ['fetch', '--quiet', 'origin', lock.rev], checkout);
    run('git', ['checkout', '--quiet', '--detach', lock.rev], checkout);
  }
  const head = execFileSync('git', ['rev-parse', 'HEAD'], { cwd: checkout, encoding: 'utf8' }).trim();
  if (head !== lock.rev) {
    console.warn(`shipped: ${checkout} is at ${head}, and shipped.toml pins ${lock.rev}`);
  }
  run('cargo', ['run', '--locked', '--release', '-p', 'vorn-pack', '--', ...lock.extensions.map((e) => e.path)], checkout);
  const out = join(core, 'shipped');
  mkdirSync(out, { recursive: true });
  const wrong = [];
  for (const e of lock.extensions) {
    const name = `${e.id}-${e.version}.vorn.tgz`;
    const from = join(checkout, 'dist', name);
    const sha = createHash('sha256').update(readFileSync(from)).digest('hex');
    if (sha !== e.sha256) {
      wrong.push(`${name}: built ${sha}, pinned ${e.sha256}`);
      continue;
    }
    copyFileSync(from, join(out, name));
    console.log(`shipped: ${name} ${sha}`);
  }
  if (wrong.length) {
    throw new Error(`not the packs shipped.toml pins:\n${wrong.join('\n')}`);
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    main();
  } catch (err) {
    console.error(`shipped: ${err.message}`);
    process.exit(1);
  }
}
