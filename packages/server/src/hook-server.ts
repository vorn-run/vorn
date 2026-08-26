import http from 'node:http'
import fs from 'node:fs'
import path from 'node:path'
import os from 'node:os'
import crypto from 'node:crypto'
import { EventEmitter } from 'node:events'
import { HookEvent } from '@vornrun/shared/types'
import log from './logger'
import { HOOK_OWNER_FILE, mayClaimHooks, pidIsAlive, readHookOwnerFile } from './hook-ownership'

const PORT_FILE = path.join(os.homedir(), '.vorn', 'port')
const TOKEN_FILE = path.join(os.homedir(), '.vorn', 'token')
const MAX_BODY_SIZE = 1024 * 1024 // 1 MB

export interface PendingPermission {
  requestId: string
  res: http.ServerResponse
  event: HookEvent
  createdAt: number
}

export class HookServer extends EventEmitter {
  private server: http.Server | null = null
  private pendingPermissions = new Map<string, PendingPermission>()
  private port = 0
  /**
   * Whether this process wrote the shared hook files, and so may remove them.
   *
   * A second Vorn on the same data directory still runs a hook server — it needs
   * one if it ever becomes the owner — but it does not advertise itself over the
   * instance the person is using.
   */
  private owner = false
  private authToken: string

  constructor() {
    super()
    this.authToken = crypto.randomUUID()
  }

  getPort(): number {
    return this.port
  }

  getAuthToken(): string {
    return this.authToken
  }

  // Try a fixed preferred port first so settings.json stays stable across
  // restarts (Claude sessions read the URL once and keep using it).
  // Falls back to an OS-assigned port if the preferred one is taken.
  private static readonly PREFERRED_PORT = 56432

  /** Verify bearer token from Authorization header */
  private authenticate(req: http.IncomingMessage, res: http.ServerResponse): boolean {
    const authHeader = req.headers.authorization
    if (!authHeader || authHeader !== `Bearer ${this.authToken}`) {
      res.writeHead(401, { 'Content-Type': 'application/json' })
      res.end(JSON.stringify({ error: 'Unauthorized' }))
      return false
    }
    return true
  }

  start(): Promise<number> {
    return new Promise((resolve, reject) => {
      this.server = http.createServer((req, res) => {
        if (req.method !== 'POST') {
          res.writeHead(404)
          res.end()
          return
        }

        // Authenticate all requests with bearer token
        if (!this.authenticate(req, res)) return

        let body = ''
        let bodySize = 0
        let aborted = false

        req.on('data', (chunk: Buffer) => {
          bodySize += chunk.length
          if (bodySize > MAX_BODY_SIZE) {
            aborted = true
            res.writeHead(413, { 'Content-Type': 'application/json' })
            res.end(JSON.stringify({ error: 'Request body too large' }))
            req.destroy()
            return
          }
          body += chunk
        })
        req.on('end', () => {
          if (aborted) return
          try {
            const event = JSON.parse(body) as HookEvent
            this.handleEvent(event, res)
          } catch {
            res.writeHead(400)
            res.end()
          }
        })
      })

      const tryListen = (port: number): void => {
        this.server!.listen(port, '127.0.0.1', () => {
          const addr = this.server!.address()
          if (typeof addr === 'object' && addr) {
            this.port = addr.port
            this.claim()
            resolve(this.port)
          } else {
            reject(new Error('Failed to get server address'))
          }
        })
      }

      this.server.once('error', (err: NodeJS.ErrnoException) => {
        if (err.code === 'EADDRINUSE') {
          // Preferred port taken -- fall back to OS-assigned
          log.info(`[hooks] port ${HookServer.PREFERRED_PORT} in use, falling back to random port`)
          this.server!.removeAllListeners('error')
          this.server!.on('error', reject)
          tryListen(0)
        } else {
          reject(err)
        }
      })

      tryListen(HookServer.PREFERRED_PORT)
    })
  }

  private handleEvent(event: HookEvent, res: http.ServerResponse): void {
    if (event.hook_event_name === 'PermissionRequest') {
      const requestId = crypto.randomUUID()
      const pending: PendingPermission = { requestId, res, event, createdAt: Date.now() }
      this.pendingPermissions.set(requestId, pending)
      log.info(
        `[hooks] permission stored: requestId=${requestId} tool=${event.tool_name} pending=${this.pendingPermissions.size}`
      )

      // Clean up if connection closes before we respond (e.g. Claude timeout)
      res.on('close', () => {
        if (this.pendingPermissions.has(requestId)) {
          log.info(
            `[hooks] permission connection closed before response: requestId=${requestId} (Claude may have timed out)`
          )
          this.pendingPermissions.delete(requestId)
          this.emit('permission-cancelled', requestId)
        }
      })

      this.emit('permission-request', { requestId, event })
    } else {
      // Fire-and-forget: respond immediately
      res.writeHead(200, { 'Content-Type': 'application/json' })
      res.end('{}')
      this.emit('hook-event', event)
    }
  }

  /** Immediately release a request with no decision -- Claude uses its own default handling */
  passthroughPermission(requestId: string): void {
    const pending = this.pendingPermissions.get(requestId)
    if (!pending) return
    this.pendingPermissions.delete(requestId)
    const { res } = pending
    if (res.writableEnded) return
    res.writeHead(200, { 'Content-Type': 'application/json' })
    res.end('{}')
  }

  resolvePermission(
    requestId: string,
    allow: boolean,
    extra?: { updatedPermissions?: unknown[]; updatedInput?: unknown }
  ): void {
    log.info(
      `[hooks] resolvePermission: requestId=${requestId} allow=${allow} pending=${this.pendingPermissions.size} found=${this.pendingPermissions.has(requestId)}`
    )
    const pending = this.pendingPermissions.get(requestId)
    if (!pending) {
      log.info(`[hooks] resolvePermission: requestId not found in pending map`)
      return
    }

    this.pendingPermissions.delete(requestId)
    const { res } = pending

    if (res.writableEnded) return

    const decision: Record<string, unknown> = { behavior: allow ? 'allow' : 'deny' }
    if (allow && extra?.updatedPermissions?.length) {
      decision.updatedPermissions = extra.updatedPermissions
    }
    if (allow && extra?.updatedInput) {
      decision.updatedInput = extra.updatedInput
    }

    const body = JSON.stringify({
      hookSpecificOutput: {
        hookEventName: 'PermissionRequest',
        decision
      }
    })

    res.writeHead(200, {
      'Content-Type': 'application/json',
      'Content-Length': Buffer.byteLength(body)
    })
    res.end(body)
  }

  getPendingPermissions(): PendingPermission[] {
    return Array.from(this.pendingPermissions.values())
  }

  cancelSessionPermissions(sessionId: string): void {
    const toCancel = Array.from(this.pendingPermissions.entries()).filter(
      ([, pending]) => pending.event.session_id === sessionId
    )
    for (const [requestId, pending] of toCancel) {
      this.pendingPermissions.delete(requestId)
      log.info(
        `[hooks] cancelling stale permission for session ${sessionId}: requestId=${requestId}`
      )
      if (!pending.res.destroyed) {
        pending.res.destroy()
      }
      this.emit('permission-cancelled', requestId)
    }
  }

  /** Whether this instance is the one registered in `~/.claude/settings.json`. */
  ownsRegistration(): boolean {
    return this.owner
  }

  /**
   * Take the shared hook files, unless a live instance already holds them.
   *
   * Everything a hook needs to reach this server — the port, the token, and the
   * settings entry that names both — describes exactly one process. Writing them
   * while another Vorn is running is what silently redirected its hooks here.
   */
  private claim(): void {
    const owner = readHookOwnerFile()
    this.owner = mayClaimHooks({ owner, selfPid: process.pid, isAlive: pidIsAlive })

    if (!this.owner) {
      log.info(
        `[hooks] another Vorn (pid ${owner?.pid}) owns the hook registration on port ` +
          `${owner?.port}, so this server listens on ${this.port} without claiming it`
      )
      return
    }

    this.writeOwnerFile()
    this.writePortFile()
    this.writeTokenFile()
  }

  private writeOwnerFile(): void {
    const dir = path.dirname(HOOK_OWNER_FILE)
    if (!fs.existsSync(dir)) fs.mkdirSync(dir, { recursive: true })
    fs.writeFileSync(
      HOOK_OWNER_FILE,
      JSON.stringify({ port: this.port, pid: process.pid }),
      'utf-8'
    )
  }

  private writePortFile(): void {
    const dir = path.dirname(PORT_FILE)
    if (!fs.existsSync(dir)) {
      fs.mkdirSync(dir, { recursive: true })
    }
    fs.writeFileSync(PORT_FILE, String(this.port), 'utf-8')
  }

  private writeTokenFile(): void {
    const dir = path.dirname(TOKEN_FILE)
    if (!fs.existsSync(dir)) {
      fs.mkdirSync(dir, { recursive: true })
    }
    fs.writeFileSync(TOKEN_FILE, this.authToken, { encoding: 'utf-8', mode: 0o600 })
  }

  stop(): void {
    // Deny all pending permissions
    const pendingIds = Array.from(this.pendingPermissions.keys())
    for (const id of pendingIds) {
      this.resolvePermission(id, false)
    }

    this.server?.close()
    this.server = null

    // Only what we wrote, confirmed against the record rather than against a flag
    // we set at startup. These files name one live server, and deleting another
    // instance's copies left the running app advertising a port and a token that
    // nothing was listening on -- so if the record has moved on, it is not ours
    // to clear.
    const owner = readHookOwnerFile()
    if (!this.owner) return
    this.owner = false
    if (owner && owner.pid !== process.pid) {
      log.info(
        `[hooks] the registration is now held by pid ${owner.pid}, so this server ` +
          'left the shared files alone'
      )
      return
    }

    for (const file of [PORT_FILE, TOKEN_FILE, HOOK_OWNER_FILE]) {
      try {
        if (fs.existsSync(file)) {
          fs.unlinkSync(file)
        }
      } catch {
        /* ignore */
      }
    }
  }
}

export const hookServer = new HookServer()
