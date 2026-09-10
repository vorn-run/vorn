import { BrowserWindow, app, session, type Session } from 'electron'
import { readdir, rm } from 'node:fs/promises'
import path from 'node:path'
import { withinOrigins } from '@vornrun/shared/connector-origins'
import {
  IPC,
  connectionPartition,
  type SdkBrowserSignIn,
  type SourceConnection
} from '../shared/types'
import type { ServerBridge } from './server/server-bridge'
import {
  MAX_SESSION_BODY,
  fetchScript,
  identityFrom,
  plainUserAgent,
  staleConnectionFolders,
  type SessionAnswer,
  type SessionRequest
} from './connection-session-script'
import log from './logger'

const CHECK_EVERY_MS = 2_000
/** Long enough to type a password and pass a second factor. */
const SIGN_IN_TIMEOUT_MS = 10 * 60_000

export interface SignInResult {
  ok: boolean
  identity?: string
  message?: string
}

const prepared = new Set<string>()
/** Hidden pages, one per connection and origin, that make the signed-in calls. */
const runners = new Map<string, BrowserWindow>()
const signInWindows = new Map<string, BrowserWindow>()
const signing = new Map<string, Promise<SignInResult>>()

function webPreferences(connectionId: string): Electron.WebPreferences {
  return {
    partition: connectionPartition(connectionId),
    sandbox: true,
    contextIsolation: true,
    nodeIntegration: false
  }
}

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

async function runner(connectionId: string, origin: string): Promise<BrowserWindow> {
  const key = `${connectionId} ${origin}`
  const existing = runners.get(key)
  if (existing && !existing.isDestroyed() && originOf(existing.webContents.getURL()) === origin) {
    return existing
  }
  existing?.destroy()
  profile(connectionId)
  const win = new BrowserWindow({
    show: false,
    webPreferences: { ...webPreferences(connectionId), backgroundThrottling: false }
  })
  runners.set(key, win)
  win.on('closed', () => {
    if (runners.get(key) === win) runners.delete(key)
  })
  // A small same-origin page to stand on, so every call is one the site's own page could make.
  await win.loadURL(`${origin}/robots.txt`)
  return win
}

function originOf(url: string): string | null {
  try {
    return new URL(url).origin
  } catch {
    return null
  }
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
  const win = await runner(connectionId, origin)
  if (originOf(win.webContents.getURL()) !== origin) {
    throw new Error(`The signed-in window for ${origin} ended up somewhere else`)
  }
  const answer = (await win.webContents.executeJavaScript(
    fetchScript(request),
    true
  )) as SessionAnswer
  return { ...answer, body: answer.body.slice(0, MAX_SESSION_BODY) }
}

/** Whether the connection is signed in, and as whom, from the check the connector declared. */
export async function checkSession(
  connectionId: string,
  browser: SdkBrowserSignIn
): Promise<{ signedIn: boolean; identity: string | null }> {
  const answer = await fetchInSession(connectionId, browser.origins, {
    url: browser.check.url,
    method: 'GET',
    headers: { accept: 'application/json' }
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
  if (!target) {
    return { ok: false, message: 'This connection does not sign in through a Vorn window.' }
  }
  const { browser } = target
  if (link !== undefined && !withinOrigins(browser.origins, link)) {
    return { ok: false, message: `That link is not on ${browser.origins.join(', ')}.` }
  }

  profile(connectionId)
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
      () => finish({ ok: false, message: 'Sign-in timed out. Try again.' }),
      SIGN_IN_TIMEOUT_MS
    )
    win.on('closed', () =>
      finish({ ok: false, message: 'The sign-in window closed before anyone signed in.' })
    )
  })
}

/** Drop the connection's windows and everything its profile holds. */
export async function forget(connectionId: string): Promise<void> {
  for (const [key, win] of runners) {
    if (!key.startsWith(`${connectionId} `)) continue
    runners.delete(key)
    if (!win.isDestroyed()) win.destroy()
  }
  signInWindows.get(connectionId)?.destroy()
  const ses = session.fromPartition(connectionPartition(connectionId))
  await ses.clearStorageData()
  await ses.clearCache()
}

export async function signOut(bridge: ServerBridge, connectionId: string): Promise<void> {
  await forget(connectionId)
  await bridge.request('connection:signedOut', connectionId)
}

/** Delete the profile folders of connections that no longer exist; Electron has no call for it. */
async function sweepConnectionProfiles(bridge: ServerBridge): Promise<void> {
  const root = path.join(app.getPath('userData'), 'Partitions')
  const folders = await readdir(root).catch(() => [] as string[])
  if (!folders.some((name) => name.startsWith('vorn-connection-'))) return
  const connections = await bridge.request<SourceConnection[]>(IPC.CONNECTION_LIST, {
    connectorId: undefined
  })
  for (const folder of staleConnectionFolders(
    folders,
    connections.map((c) => c.id)
  )) {
    await rm(path.join(root, folder), { recursive: true, force: true }).catch((err) =>
      log.warn({ err, folder }, '[sign-in] could not remove a stale profile')
    )
  }
}

export function installConnectionSessions(bridge: ServerBridge): void {
  setTimeout(() => {
    void sweepConnectionProfiles(bridge).catch((err) =>
      log.warn({ err }, '[sign-in] profile sweep failed')
    )
  }, 2_000)
}
