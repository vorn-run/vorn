import { describe, it, expect } from 'vitest'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { HANDOFF_MANIFEST_VERSION } from '@vornrun/shared/protocol'
import {
  readManifest,
  writeManifest,
  manifestPath,
  FIRST_PTY_SLOT,
  type HandoffManifest
} from '../packages/server/src/handoff/manifest'

/**
 * Null means "do not serve", and the replacement exits so the outgoing server
 * resumes. Reading a damaged manifest as "no panes" is work disappearing.
 */
function tmp(): string {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'vorn-manifest-'))
}

function valid(): HandoffManifest {
  return {
    version: HANDOFF_MANIFEST_VERSION,
    donorPid: 123,
    createdAt: Date.now(),
    panes: [
      {
        slot: FIRST_PTY_SLOT,
        session: { id: 'abc' } as HandoffManifest['panes'][0]['session'],
        pid: 999,
        cols: 80,
        rows: 24
      }
    ]
  }
}

describe('the handoff manifest', () => {
  it('round-trips what the replacement needs', () => {
    const dir = tmp()
    const at = manifestPath(dir, 123)
    writeManifest(at, valid())
    expect(readManifest(at)?.panes[0]).toMatchObject({ slot: FIRST_PTY_SLOT, pid: 999 })
  })

  it('lands whole or not at all', () => {
    // Written to a scratch name and renamed. A replacement that read a partly
    // written file would adopt some terminals and silently drop the rest.
    const dir = tmp()
    const at = manifestPath(dir, 123)
    writeManifest(at, valid())
    expect(fs.readdirSync(dir)).toEqual([path.basename(at)])
  })

  it('refuses a manifest from a contract it does not know', () => {
    const dir = tmp()
    const at = manifestPath(dir, 1)
    fs.writeFileSync(at, JSON.stringify({ ...valid(), version: 99 }))
    expect(readManifest(at)).toBeNull()
  })

  it('refuses a pane pointing at a slot that cannot hold one', () => {
    // 0 to 2 are the replacement's own stdio and 3 is its channel. A manifest
    // naming one of those is describing something other than a terminal.
    const dir = tmp()
    const at = manifestPath(dir, 1)
    const bad = valid()
    bad.panes[0]!.slot = 2
    fs.writeFileSync(at, JSON.stringify(bad))
    expect(readManifest(at)).toBeNull()
  })

  it('refuses a pane with no session behind it', () => {
    const dir = tmp()
    const at = manifestPath(dir, 1)
    const bad = valid()
    delete (bad.panes[0] as { session?: unknown }).session
    fs.writeFileSync(at, JSON.stringify(bad))
    expect(readManifest(at)).toBeNull()
  })

  it('refuses a file that is not a manifest at all', () => {
    const dir = tmp()
    const at = manifestPath(dir, 1)
    fs.writeFileSync(at, 'half a jso')
    expect(readManifest(at)).toBeNull()
    // And an absent one, which is the same answer for the same reason.
    expect(readManifest(path.join(dir, 'nothing-here.json'))).toBeNull()
  })
})
