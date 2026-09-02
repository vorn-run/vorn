import { afterEach, describe, expect, it } from 'vitest'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { create } from 'tar'
import {
  describePack,
  inspectPack,
  installPack,
  resetStagedPacks
} from '../packages/server/src/connectors/packs'

const temps: string[] = []

function tempDir(): string {
  const dir = mkdtempSync(join(tmpdir(), 'vorn-pack-auth-'))
  temps.push(dir)
  return dir
}

afterEach(() => {
  resetStagedPacks()
  while (temps.length > 0) rmSync(temps.pop() as string, { recursive: true, force: true })
})

/** A connector that borrows a login and takes an argument with known choices. */
const MANIFEST = {
  id: 'acme',
  name: 'Acme',
  version: '1.2.0',
  auth: {
    rung: 'cli',
    probe: { command: 'acme', args: ['auth', 'status'] },
    borrow: { tokenArgs: ['auth', 'token'] }
  },
  triggers: [
    {
      type: 'newTicket',
      label: 'New ticket',
      setup: { filters: { pollTool: 'poll_newTicket' }, env: [] }
    }
  ],
  actions: [
    {
      type: 'closeTicket',
      label: 'Close ticket',
      inputs: [
        { key: 'id', label: 'Ticket', type: 'string', required: true },
        {
          key: 'reason',
          label: 'Reason',
          type: 'select',
          required: false,
          options: [{ value: 'fixed' }, { value: 'wontfix', label: "Won't fix" }],
          loadOptions: 'reasons'
        }
      ]
    }
  ]
}

async function packWith(manifest: unknown): Promise<string> {
  const staging = tempDir()
  writeFileSync(join(staging, 'manifest.json'), JSON.stringify(manifest))
  writeFileSync(join(staging, 'index.js'), 'process.stdin.resume()\n')
  const file = join(tempDir(), 'pack.tgz')
  await create({ gzip: true, file, cwd: staging }, ['manifest.json', 'index.js'])
  return file
}

describe('what a pack tells the app about signing in', () => {
  it('reaches the sheet that asks to keep it, and the record of what was kept', async () => {
    const root = tempDir()
    const file = await packWith(MANIFEST)

    const inspected = await inspectPack({ kind: 'file', path: file }, { root })
    expect(inspected.ok).toBe(true)
    if (!inspected.ok) return
    expect(inspected.preview.auth).toEqual(MANIFEST.auth)

    const installed = await installPack(
      { kind: 'staged', token: inspected.preview.token },
      { root }
    )
    expect(installed.ok).toBe(true)
    expect(describePack('acme', { root })?.auth).toEqual(MANIFEST.auth)
  })

  it('says nothing where the connector said nothing, rather than guessing a rung', async () => {
    const root = tempDir()
    // JSON drops the undefined, so the packed manifest simply has no auth key.
    const file = await packWith({ ...MANIFEST, auth: undefined })

    const inspected = await inspectPack({ kind: 'file', path: file }, { root })
    expect(inspected.ok).toBe(true)
    if (!inspected.ok) return
    expect(inspected.preview.auth).toBeUndefined()
  })
})

describe("what a pack's actions take", () => {
  it('survives the round trip, so a step can name its arguments', async () => {
    const root = tempDir()
    const file = await packWith(MANIFEST)

    const inspected = await inspectPack({ kind: 'file', path: file }, { root })
    expect(inspected.ok).toBe(true)
    if (!inspected.ok) return
    expect(inspected.preview.actions[0].inputs).toEqual(MANIFEST.actions[0].inputs)

    await installPack({ kind: 'staged', token: inspected.preview.token }, { root })
    const onDisk = describePack('acme', { root })
    expect(onDisk?.actions[0].inputs?.[1]).toEqual(MANIFEST.actions[0].inputs[1])
  })
})
