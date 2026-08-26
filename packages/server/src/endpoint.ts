import fs from 'fs'
import net from 'net'
import path from 'path'
import crypto from 'crypto'
import log from './logger'
import { ENDPOINT_FILENAME } from '@vornrun/shared/protocol'

/**
 * Owning the name a machine's server answers on.
 *
 * A port has no name to own. Two servers reaching for one are settled by
 * whichever bound first, and nothing about that is visible to a third party
 * deciding whether to start a third. A filesystem entry can be owned, and the
 * kernel offers exactly the primitive that makes it a test-and-set:
 *
 * - `link()` fails with `EEXIST` rather than replacing, so it asks a question.
 * - `rename()` within a filesystem is atomic and leaves no moment where the
 *   destination name is absent, so it answers one.
 *
 * A bound `AF_UNIX` listener is identified by its inode, not by the path it was
 * bound to, so any directory entry resolving to that inode reaches it. That is
 * what makes the sequence below possible: bind a private name, then move it into
 * place only after proving the place is free.
 *
 * It also retires the rule that would otherwise be the easiest to break. libuv
 * unlinks the path it bound when the handle closes, with no ownership check —
 * so a server binding the canonical name directly deletes a live replacement's
 * socket on its way out. Binding a scratch name means the path libuv remembers
 * is one that no longer exists by then, and `close()` becomes structurally
 * incapable of removing the endpoint. Verified on darwin/APFS before this was
 * written: link on a socket inode, EEXIST on a taken name, connect through the
 * link, and close leaving the canonical entry intact on both paths.
 *
 * Two invariants hold the whole thing up:
 *
 * > Only a process publishing itself onto the canonical endpoint may mutate that
 * > entry, and only by replacing an entry it has itself just proven dead.
 * >
 * > No actor removes a name it did not create.
 *
 * There is deliberately no sweeper. Deciding whether somebody else's leftover is
 * safe to delete is the question this design exists to retire, and a sweeper
 * reintroduces it with worse information.
 */

/** Long enough for a busy accept queue, short enough not to stall a launch. */
const PROBE_TIMEOUT_MS = 1_000

/**
 * How many times to re-look when the entry changes hands mid-check.
 *
 * Bounded rather than infinite: something is churning that name, and the right
 * answer to a contended endpoint is to stand down, not to fight for it.
 */
const RECHECK_LIMIT = 3

/**
 * `sun_path` is 104 bytes on darwin and 108 on linux, including the terminator.
 *
 * Close to the smaller of the two rather than well under it, because what gets
 * measured is now the scratch name that is actually bound rather than the shorter
 * canonical one. The old margin was covering the thirteen bytes this check used
 * to ignore, and once that was fixed the margin was pure cost -- it turned a
 * macOS temp directory, which is where every process-level test runs, into a
 * machine that silently hosts no endpoint.
 */
const MAX_SOCKET_PATH = 100

export function endpointPath(dataDir: string): string {
  return path.join(dataDir, ENDPOINT_FILENAME)
}

/** A private name to bind, with enough randomness that nothing collides with it. */
export function scratchPathFor(canonical: string): string {
  return `${canonical}.${crypto.randomBytes(6).toString('hex')}`
}

/**
 * Whether this machine can host the endpoint at all.
 *
 * Every no here is a downgrade to TCP-only, never a failed startup: the socket
 * is additive, and a server nobody can reach is worse than a race nobody has
 * lost yet.
 */
export function canHostEndpoint(dataDir: string): { ok: boolean; why?: string } {
  // Node maps `listen({ path })` to a named pipe on win32, and a pipe has no
  // filesystem entry to test-and-set. Windows keeps the port file.
  if (process.platform === 'win32') return { ok: false, why: 'not a POSIX filesystem' }

  // Measured on a scratch name, not the canonical one, because the scratch name
  // is what gets bound -- and it is thirteen bytes longer. Checking the shorter
  // path would pass a directory whose socket then fails to bind at runtime, which
  // is the exact failure this guard exists to turn into a clean downgrade.
  // Generated rather than assumed, so the two cannot drift apart.
  const bound = scratchPathFor(endpointPath(dataDir))
  if (Buffer.byteLength(bound) > MAX_SOCKET_PATH) {
    return { ok: false, why: `path is ${Buffer.byteLength(bound)} bytes` }
  }

  // The directory is the access control, and it has to be private -- not merely
  // unwritable by others.
  //
  // Writable was the obvious half: anyone who can write here can rename over the
  // endpoint. Readable and traversable matters too, and it took a review to see
  // it. On darwin a socket's own mode bits are not consulted on connect, so the
  // 0600 this code sets on the socket protects nothing; a directory another user
  // can traverse is a socket another user can connect to. And the greeting is
  // sent before authentication -- deliberately, since that is what lets a desktop
  // learn who is serving -- so a stranger who connects is handed this server's
  // identity, `dataDir` included, which names the account.
  //
  // Loopback bounded that disclosure for TCP peers. A unix peer is judged local
  // by the directory alone, so the directory is where the bound has to be.
  // `database.ts` creates it 0700, so an ordinary install passes and only one
  // that predates that, or a umask accident, is turned away -- to TCP-only,
  // never to a failed start.
  try {
    if (fs.statSync(dataDir).mode & 0o077) {
      // Narrowed rather than refused, where this process is allowed to. The
      // directory holds this server's credential, its database and its tokens,
      // `database.ts` already means it to be 0700, and `mode` on mkdir applies
      // only at creation -- so an install made before that, or under a loose
      // umask, has been sitting wider than intended all along. Tightening it is
      // the fix for that, not a workaround for this check. Never widened: this
      // only ever removes bits.
      fs.chmodSync(dataDir, 0o700)
      log.info({ dataDir }, '[endpoint] tightened the data directory to 0700')
    }
    const mode = fs.statSync(dataDir).mode & 0o777
    if (mode & 0o077) return { ok: false, why: `directory is mode ${mode.toString(8)}` }
  } catch (err) {
    return { ok: false, why: `cannot secure the data directory: ${(err as Error).message}` }
  }

  return { ok: true }
}

export type Liveness = 'alive' | 'dead' | 'unknown'

/**
 * Ask whether anything is serving this name.
 *
 * Three answers, never two. Only a completed connection proves it is served, and
 * only `ECONNREFUSED` or `ENOENT` prove it is not — a socket file whose server
 * has gone refuses, and a name with nothing behind it is absent. Everything else
 * proves nothing: `EACCES` and `EPERM` say we may not ask, `EAGAIN` says the
 * accept queue is full, which only a running server can manage, and a timeout
 * says the server is busy or wedged.
 *
 * The whitelist is the two that mean death, and everything unlisted defaults to
 * alive. Enumerating failures the other way round is how a starting server
 * deletes an endpoint still holding every terminal on the machine.
 */
export function probeEndpoint(socketPath: string, timeoutMs = PROBE_TIMEOUT_MS): Promise<Liveness> {
  return new Promise((resolve) => {
    const socket = net.connect(socketPath)
    let settled = false
    const done = (answer: Liveness): void => {
      if (settled) return
      settled = true
      socket.destroy()
      resolve(answer)
    }

    socket.setTimeout(timeoutMs, () => done('unknown'))
    socket.once('connect', () => done('alive'))
    // `on`, not `once`. Destroying a socket the moment it connects can draw a
    // second error afterwards -- an ECONNRESET as the peer's reply lands on a
    // half-closed pipe -- and a handler that unhooked itself leaves that one
    // unhandled, which takes down the process. A probe whose whole job is to keep
    // a server safe must not be able to kill the one asking.
    socket.on('error', (err: NodeJS.ErrnoException) => {
      done(err.code === 'ECONNREFUSED' || err.code === 'ENOENT' ? 'dead' : 'unknown')
    })
  })
}

interface Entry {
  dev: number
  ino: number
}

function identify(target: string): Entry | null {
  try {
    const stat = fs.lstatSync(target)
    // Not a socket, or a symlink pointing anywhere at all: this is not something
    // this process put there and not something it can identify, so it is not
    // something it may replace.
    if (!stat.isSocket()) return null
    return { dev: stat.dev, ino: stat.ino }
  } catch {
    return null
  }
}

const same = (a: Entry | null, b: Entry | null): boolean =>
  a !== null && b !== null && a.dev === b.dev && a.ino === b.ino

export type ClaimOutcome =
  | { held: true }
  /** `because` is for the log, and for the launcher deciding what to do next. */
  | { held: false; because: string }

/**
 * Move a bound scratch name onto the canonical one, if the canonical one is free
 * or its holder is provably gone.
 *
 * `probe` is injected so the decision can be tested against each of the three
 * answers without a real server on the other end.
 */
export async function claimEndpoint(
  scratch: string,
  canonical: string,
  probe: (target: string) => Promise<Liveness> = probeEndpoint
): Promise<ClaimOutcome> {
  const mine = identify(scratch)
  if (!mine) return { held: false, because: 'the scratch name is not a bound socket' }

  /** Ours. Remove the scratch name, which is a name this process created -- not
   *  the forbidden unlink-then-link, which is about the canonical entry. After
   *  this, libuv's unlink at close has nothing to find on either path. */
  const took = (): ClaimOutcome => {
    fs.rmSync(scratch, { force: true })
    return { held: true }
  }

  for (let attempt = 0; attempt < RECHECK_LIMIT; attempt++) {
    // The test-and-set, and the whole loop exists to come back to it. `link`
    // first and never a bare `rename`, because rename replaces whatever it finds
    // and would let a starting server destroy a healthy one without ever asking.
    try {
      fs.linkSync(scratch, canonical)
      return took()
    } catch (err) {
      const code = (err as NodeJS.ErrnoException).code
      if (code !== 'EEXIST') return { held: false, because: `link failed: ${code}` }
    }

    if (!fs.existsSync(canonical)) {
      // Taken a moment ago and gone now. A server that bound the canonical path
      // directly -- which is what this codebase did before the scratch name --
      // unlinks it on close, so the name really can free itself between the
      // EEXIST and this look. It is free, so go and take it rather than standing
      // down from an endpoint nobody holds.
      continue
    }

    const before = identify(canonical)
    if (!before) {
      // It exists and is not a socket. Not something this process put there, not
      // something it can identify, and so not something it may replace.
      return { held: false, because: 'the endpoint is not a socket this process can read' }
    }

    const liveness = await probe(canonical)
    if (liveness !== 'dead') return { held: false, because: `the incumbent is ${liveness}` }

    // Between the probe and the rename, the entry may have become somebody
    // else's -- a third server that also found it dead and got there first. The
    // inode is the identity; a path comparison would not notice, and
    // `birthtimeMs` cannot be trusted to (Node's own docs say it sometimes holds
    // the ctime, filesystems without a birth time report the epoch, and its
    // granularity is often coarser than the events being separated).
    if (!same(before, identify(canonical))) continue

    try {
      fs.renameSync(scratch, canonical)
    } catch (err) {
      return { held: false, because: `rename failed: ${(err as NodeJS.ErrnoException).code}` }
    }

    // The probe and the rename are two syscalls and POSIX offers no
    // rename-if-target-is-inode-X, so this is the window that cannot be closed.
    // What can be closed is the harm: a process that finds it lost the name must
    // never go on to serve as though it holds it.
    if (same(mine, identify(canonical))) return { held: true }
    return { held: false, because: 'lost the endpoint between the rename and the check' }
  }

  return { held: false, because: 'the endpoint changed hands while it was being checked' }
}

/** Remove a scratch name this process created. Never the canonical one. */
export function abandonScratch(scratch: string): void {
  try {
    fs.rmSync(scratch, { force: true })
  } catch (err) {
    log.warn({ err, scratch }, '[endpoint] could not remove our own scratch name')
  }
}
