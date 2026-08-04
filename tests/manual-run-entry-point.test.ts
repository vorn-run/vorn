import { describe, it, expect } from 'vitest'
import { readFileSync, readdirSync, statSync } from 'node:fs'
import { join } from 'node:path'

/**
 * `executeWorkflow` runs a workflow with whatever context it is handed. A UI
 * surface that calls it directly silently skips the run dialog, so a workflow
 * declaring inputs launches with `context.inputs` undefined and sends
 * `{{inputs.*}}` to the agent as literal text. That shipped once already, from
 * the workflow editor's Run button.
 *
 * `startManualRun` is the single door that gates on `needsRunPrompt`. This
 * pins the allowlist so a new surface reaching for `executeWorkflow` fails
 * here instead of in production, where the failure is invisible.
 */
const ALLOWED = new Set([
  // The dialog itself — it runs the workflow after collecting what's missing.
  'src/renderer/components/SourcePromptDialog.tsx',
  // The single manual-run entry point.
  'src/renderer/lib/workflow-menu-items.tsx',
  // Non-manual paths: no user is present to prompt.
  'src/renderer/lib/workflow-triggers.ts',
  'src/renderer/components/MissedScheduleDialog.tsx',
  // The scheduler's SCHEDULER_EXECUTE handler.
  'src/renderer/App.tsx'
])

function walk(dir: string, out: string[] = []): string[] {
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry)
    if (statSync(full).isDirectory()) walk(full, out)
    else if (/\.tsx?$/.test(entry)) out.push(full)
  }
  return out
}

describe('manual workflow runs go through one entry point', () => {
  it('has no unexpected direct callers of executeWorkflow', () => {
    const root = join(__dirname, '..')
    const callers = walk(join(root, 'src/renderer'))
      .filter((f) => {
        const src = readFileSync(f, 'utf8')
        // The import site is what matters; the definition lives elsewhere.
        return /\bexecuteWorkflow\b/.test(src) && /from '.*workflow-execution'/.test(src)
      })
      .map((f) => f.slice(root.length + 1).replace(/\\/g, '/'))

    expect(new Set(callers)).toEqual(ALLOWED)
  })
})
