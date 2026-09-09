import { describe, it, expect } from 'vitest'
import { readFileSync, readdirSync, statSync } from 'node:fs'
import { join } from 'node:path'

/**
 * A manual run has to go through one door.
 *
 * `startManualRun` is the door: it gates on `needsRunPrompt`, so a workflow
 * declaring inputs collects them before it runs. A surface that asks the server
 * to run a workflow directly skips that, and the failure is invisible —
 * `{{inputs.*}}` reaches the agent as literal text. That shipped once already,
 * from the editor's Run button.
 *
 * Since execution moved to the server, the thing to pin is `runWorkflow`, the
 * request that starts one. The engine itself is no longer importable from a
 * window at all, which the second case here holds to.
 */
const ALLOWED = new Set([
  // The dialog itself — it runs the workflow after collecting what is missing.
  'src/renderer/components/SourcePromptDialog.tsx',
  // The single manual-run entry point.
  'src/renderer/lib/workflow-menu-items.tsx',
  // Recovery for a schedule missed while the app was closed; no user to prompt.
  'src/renderer/components/MissedScheduleDialog.tsx'
])

function walk(dir: string, out: string[] = []): string[] {
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry)
    if (statSync(full).isDirectory()) walk(full, out)
    else if (/\.tsx?$/.test(entry)) out.push(full)
  }
  return out
}

function rendererFiles(): { path: string; source: string }[] {
  const root = join(__dirname, '..')
  return walk(join(root, 'src/renderer')).map((f) => ({
    path: f.slice(root.length + 1).replace(/\\/g, '/'),
    source: readFileSync(f, 'utf8')
  }))
}

describe('manual workflow runs go through one entry point', () => {
  it('has no unexpected callers of window.api.runWorkflow', () => {
    const callers = rendererFiles()
      .filter(({ source }) =>
        /window\.api\s*\n?\s*\.?runWorkflow\b|api\.runWorkflow\(/.test(source)
      )
      .map(({ path }) => path)

    expect(new Set(callers)).toEqual(ALLOWED)
  })

  it('leaves the engine to the server', () => {
    const importers = rendererFiles()
      .filter(({ source }) => /from '.*workflows\/engine'/.test(source))
      .map(({ path }) => path)

    expect(importers).toEqual([])
  })
})
