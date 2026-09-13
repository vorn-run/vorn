import { BrowserWindow, app, session, type Session } from 'electron'
import { readdir, rm, stat } from 'node:fs/promises'
import path from 'node:path'
import { withinOrigins } from '@vornrun/shared/connector-origins'
import {
  CONNECTION_PROFILE_PREFIX,
  IPC,
  connectionPartition,
  type SdkBrowserSignIn,
  type SessionAnswer,
  type SessionRequest,
  type SourceConnection
} from '../shared/types'
import type { ServerBridge } from './server/server-bridge'
import {
  MAX_SESSION_BODY,
  fetchScript,
  identityFrom,
  plainUserAgent,
  staleConnectionFolders
} from './connection-session-script'
import log from './logger'

const CHECK_EVERY_MS = 2_000
/** A runner nobody has used for this long is closed; the next call opens it again. */
const RUNNER_IDLE_MS = 60_000
/** Long enough to type a password and pass a second factor. */
const SIGN_IN_TIMEOUT_MS = 10 * 60_000

export interface SignInResult {
  ok: boolean
  identity?: string
  error?: string
}

const prepared = new Set<string>()
/** A hidden page, one per connection and origin, that makes the signed-in calls. */
interface Runner {
  win: BrowserWindow
  ready: Promise<void>
  /** Calls in flight; the page only starts idling once the last one ends. */
  busy: number
  idle?: NodeJS.Timeout
}
const runners = new Map<string, Runner>()
const signInWindows = new Map<string, BrowserWindow>()
const signing = new Map<string, Promise<SignInResult>>()

/** The connection's profile, set up once: a plain user agent, no permissions, no downloads. */
function profile(connectionId: string): Session {
  const partition = connectionPartition(connectionId)
  const ses = session.fromPartition(partition)
  if (!prepared.has(partition)) {
    prepared.add(partition)
    ses.setUserAgent(plainUserAgent(ses.getUserAgent()))
    ses.setPermissionRequestHandler((_contents, _permission, callback) => callback(false))
    ses.setPermissionCheckHandler(() => false)
    ses.on('will-download', (event) => event.preventDefault())
  }
  return ses
}

function webPreferences(connectionId: string): Electron.WebPreferences {
  profile(connectionId)
  return {
    partition: connectionPartition(connectionId),
    sandbox: true,
    contextIsolation: true,
    nodeIntegration: false
  }
}

function originOf(url: string): string | null {
  try {
    return new URL(url).origin
  } catch {
    return null
  }
}

function closeRunner(key: string, runner: Runner): void {
  if (runners.get(key) === runner) runners.delete(key)
  clearTimeout(runner.idle)
  if (!runner.win.isDestroyed()) runner.win.destroy()
}

/** The page for this origin, opened once and shared by every call that arrives while it loads. */
function runnerFor(key: string, connectionId: string, origin: string): Runner {
  const existing = runners.get(key)
  if (existing && !existing.win.isDestroyed()) return existing
  const win = new BrowserWindow({
    show: false,
    webPreferences: { ...webPreferences(connectionId), backgroundThrottling: false }
  })
  // A small same-origin page to stand on, so every call is one the site's own page could make.
  const runner: Runner = { win, ready: win.loadURL(`${origin}/robots.txt`), busy: 0 }
  runners.set(key, runner)
  win.on('closed', () => {
    if (runners.get(key) === runner) runners.delete(key)
  })
  runner.ready.catch(() => closeRunner(key, runner))
  return runner
}

/** Make one call inside the connection's signed-in profile, from a page on the call's own origin. */
export async function fetchInSession(
  connectionId: string,
  origins: readonly string[],
  request: SessionRequest
): Promise<SessionAnswer> {
  if (!withinOrigins(origins, request.url)) {
    throw new Error(`${request.url} is not on one of this connection's origins`)
  }
  const origin = originOf(request.url)!
  const key = `${connectionId} ${origin}`
  const runner = runnerFor(key, connectionId, origin)
  clearTimeout(runner.idle)
  runner.busy++
  try {
    await runner.ready
    if (originOf(runner.win.webContents.getURL()) !== origin) {
      closeRunner(key, runner)
      throw new Error(`The signed-in window for ${origin} ended up somewhere else`)
    }
    const answer = (await runner.win.webContents.executeJavaScript(
      fetchScript(request),
      true
    )) as SessionAnswer
    return { ...answer, body: answer.body.slice(0, MAX_SESSION_BODY) }
  } finally {
    runner.busy--
    if (runner.busy === 0 && runners.get(key) === runner) {
      runner.idle = setTimeout(() => closeRunner(key, runner), RUNNER_IDLE_MS)
      runner.idle.unref()
    }
  }
}

/** Whether the connection is signed in, and as whom, from the check the connector declared. */
export async function checkSession(
  connectionId: string,
  browser: SdkBrowserSignIn
): Promise<{ signedIn: boolean; identity: string | null }> {
  const answer = await fetchInSession(connectionId, browser.origins, {
    url: browser.check.url,
    method: 'GET',
    headers: { accept: 'application/json', ...browser.check.headers }
  })
  const signedIn = answer.status >= 200 && answer.status < 300
  return { signedIn, identity: signedIn ? identityFrom(answer.body, browser.check.identity) : null }
}

/** Open the connection's sign-in window and settle when the check says someone is signed in. */
export function signIn(
  bridge: ServerBridge,
  connectionId: string,
  link?: string
): Promise<SignInResult> {
  const pending = signing.get(connectionId)
  if (pending) {
    signInWindows.get(connectionId)?.focus()
    return pending
  }
  const run = openSignIn(bridge, connectionId, link).finally(() => signing.delete(connectionId))
  signing.set(connectionId, run)
  return run
}

async function openSignIn(
  bridge: ServerBridge,
  connectionId: string,
  link?: string
): Promise<SignInResult> {
  const target = await bridge.request<{ name: string; browser: SdkBrowserSignIn } | null>(
    'connection:browserAuth',
    connectionId
  )
  if (!target)
    return { ok: false, error: 'This connection does not sign in through a Vorn window.' }
  const { browser } = target
  if (link !== undefined && !withinOrigins(browser.origins, link)) {
    return { ok: false, error: `That link is not on ${browser.origins.join(', ')}.` }
  }

  const win = new BrowserWindow({
    width: 520,
    height: 720,
    title: `Sign in · ${target.name}`,
    autoHideMenuBar: true,
    webPreferences: webPreferences(connectionId)
  })
  signInWindows.set(connectionId, win)
  // Sign-in providers open windows of their own; they share the profile so the session lands in it.
  win.webContents.setWindowOpenHandler(({ url }) =>
    url.startsWith('https://')
      ? {
          action: 'allow',
          overrideBrowserWindowOptions: {
            parent: win,
            width: 480,
            height: 640,
            autoHideMenuBar: true,
            webPreferences: webPreferences(connectionId)
          }
        }
      : { action: 'deny' }
  )
  void win
    .loadURL(link ?? browser.signInUrl)
    .catch((err) => log.warn({ err }, '[sign-in] load failed'))

  return new Promise<SignInResult>((resolve) => {
    let settled = false
    let checking = false
    const finish = (result: SignInResult): void => {
      if (settled) return
      settled = true
      clearInterval(timer)
      clearTimeout(limit)
      signInWindows.delete(connectionId)
      if (!win.isDestroyed()) win.close()
      resolve(result)
    }
    const timer = setInterval(() => {
      if (checking) return
      checking = true
      void checkSession(connectionId, browser)
        .then(async ({ signedIn, identity }) => {
          if (!signedIn) return
          await bridge.request('connection:signedIn', { connectionId, identity })
          finish({ ok: true, ...(identity && { identity }) })
        })
        .catch((err) => log.warn({ err }, '[sign-in] check failed'))
        .finally(() => {
          checking = false
        })
    }, CHECK_EVERY_MS)
    const limit = setTimeout(
      () => finish({ ok: false, error: 'Sign-in timed out. Try again.' }),
      SIGN_IN_TIMEOUT_MS
    )
    win.on('closed', () =>
      finish({ ok: false, error: 'The sign-in window closed before anyone signed in.' })
    )
  })
}

function closeRunners(connectionId?: string): void {
  for (const [key, runner] of runners) {
    if (connectionId === undefined || key.startsWith(`${connectionId} `)) closeRunner(key, runner)
  }
}

/** Close every window a connection keeps, so hidden ones never hold the app open once its own window closes. */
export function closeConnectionWindows(): void {
  closeRunners()
  for (const win of signInWindows.values()) if (!win.isDestroyed()) win.destroy()
}

function partitionsRoot(): string {
  return path.join(app.getPath('userData'), 'Partitions')
}

async function profileOnDisk(connectionId: string): Promise<boolean> {
  return stat(path.join(partitionsRoot(), `${CONNECTION_PROFILE_PREFIX}${connectionId}`)).then(
    () => true,
    () => false
  )
}

/** Drop the connection's windows and everything its profile holds. */
export async function forget(connectionId: string): Promise<void> {
  closeRunners(connectionId)
  signInWindows.get(connectionId)?.destroy()
  const partition = connectionPartition(connectionId)
  // Asking for a profile that was never made would create one just to clear it.
  if (!prepared.has(partition) && !(await profileOnDisk(connectionId))) return
  const ses = session.fromPartition(partition)
  await Promise.all([ses.clearStorageData(), ses.clearCache()])
}

export async function signOut(bridge: ServerBridge, connectionId: string): Promise<void> {
  await forget(connectionId)
  await bridge.request('connection:signedOut', connectionId)
}

/** Delete the profile folders of connections that no longer exist; Electron has no call for it. */
async function sweepConnectionProfiles(bridge: ServerBridge): Promise<void> {
  const root = partitionsRoot()
  const folders = await readdir(root).catch(() => [] as string[])
  if (!folders.some((name) => name.startsWith(CONNECTION_PROFILE_PREFIX))) return
  const connections = await bridge.request<SourceConnection[]>(IPC.CONNECTION_LIST, {
    connectorId: undefined
  })
  const stale = staleConnectionFolders(
    folders,
    connections.map((c) => c.id)
  )
  await Promise.all(
    stale.map((folder) =>
      rm(path.join(root, folder), { recursive: true, force: true }).catch((err) =>
        log.warn({ err, folder }, '[sign-in] could not remove a stale profile')
      )
    )
  )
}

/** `sweep` is off when this desktop talks to another machine's server, whose list is not this machine's. */
export function installConnectionSessions(bridge: ServerBridge, options: { sweep: boolean }): void {
  if (!options.sweep) return
  setTimeout(() => {
    void sweepConnectionProfiles(bridge).catch((err) =>
      log.warn({ err }, '[sign-in] profile sweep failed')
    )
  }, 500)
}
