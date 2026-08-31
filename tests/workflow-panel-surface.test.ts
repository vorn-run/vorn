import { describe, it, expect } from 'vitest'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'

const root = join(__dirname, '..')
const panels = join(root, 'src/renderer/components/workflow-editor/panels')
const read = (name: string): string => readFileSync(join(panels, name), 'utf8')

/** The panels docked to an edge of the editor, each a permanent region of it. */
const DOCKED = [
  'NodeConfigPanel.tsx',
  'WorkflowPropertiesPanel.tsx',
  'RunHistoryPanel.tsx',
  'StepLibrary.tsx'
] as const

/** The one panel element in each file: the root, which carries the edge border. */
const rootClasses = (source: string): string =>
  source.match(/className="([^"]*border-[lr] [^"]*)"/)?.[1] ?? ''

describe('the workflow editor panels', () => {
  it('sits on the same ground as the nodes it inspects', () => {
    // These are docked regions of the editor, not floating chrome. On the
    // overlay rung they were the lightest thing on screen and read pale beside
    // a canvas of nodes; the node rung ties a config panel to the node it is
    // describing.
    const wrong = DOCKED.filter((name) => !rootClasses(read(name)).includes('bg-surface-node'))
    expect(wrong).toEqual([])
  })

  it('leaves the overlay rung to the things that actually float', () => {
    // A dropdown opened from inside a panel does float over it, and has to stay
    // distinguishable from the panel it covers — so the rung is not wrong
    // everywhere, only on the panels themselves.
    const onOverlay = DOCKED.filter((name) =>
      rootClasses(read(name)).includes('bg-surface-overlay')
    )
    expect(onOverlay).toEqual([])

    const menu = read('NodeConfigPanel.tsx')
    expect(menu).toContain("background: 'var(--color-surface-overlay)'")
  })
})
