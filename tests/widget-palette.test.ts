import { describe, it, expect } from 'vitest'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'

const root = join(__dirname, '..')
const read = (p: string): string => readFileSync(join(root, p), 'utf8')

const widget = read('src/renderer/widget.css')

describe('the widget reads the same palette as the app', () => {
  it('imports the theme rather than keeping its own copy', () => {
    // The widget is a separate window with a separate stylesheet, which is how
    // it became the one surface still painting a waiting agent yellow. @theme
    // emits plain custom properties, so the import is all it needs — no
    // Tailwind entry, no second declaration to drift.
    expect(widget).toMatch(/@import\s+['"]\.\/theme\.css['"]/)
    expect(widget).not.toContain('--color-bronzo:')

    // And it has to be the first statement. PostCSS silently discards an
    // @import that any rule precedes — it warns rather than errors, so the
    // build stays green while every token resolves to nothing. Comments are
    // allowed ahead of it; nothing else is.
    const firstStatement = widget
      .replace(/\/\*[\s\S]*?\*\//g, '')
      .split('\n')
      .map((l) => l.trim())
      .find(Boolean)
    expect(firstStatement).toMatch(/^@import/)
  })

  it('marks the one state that is blocked on the person with the accent', () => {
    // Same rule as the session dock, the runs list and the task board: waiting
    // takes bronzo and nothing else does.
    const rule = (name: string): string =>
      widget.match(new RegExp(`\\.status-${name}\\s*\\{([^}]*)\\}`))?.[1].trim() ?? ''

    expect(rule('waiting')).toContain('var(--color-bronzo)')
    for (const other of ['running', 'idle', 'error']) {
      expect([other, rule(other).includes('bronzo')]).toEqual([other, false])
    }
  })

  it('reads every status off the shared vocabulary, not a literal', () => {
    const statuses = ['running', 'waiting', 'idle', 'error']
    const literal = statuses.filter((s) => {
      const rule = widget.match(new RegExp(`\\.status-${s}\\s*\\{([^}]*)\\}`))?.[1] ?? ''
      return /#[0-9a-fA-F]{3,8}/.test(rule)
    })
    expect(literal).toEqual([])
  })

  it('keeps no palette of its own beside the shared one', () => {
    // Four hardcoded status hues, an indigo used for focus and selection, and a
    // second spelling of the accent all lived here. A hue the app does not have
    // a name for is the thing worth catching.
    // Matched loosely on purpose: reformatting `99, 102, 241` to `99,102,241`
    // would otherwise let the same colour back in unnoticed.
    const sheet = widget.toLowerCase().replace(/\s+/g, '')
    const strays = ['#4ade80', '#facc15', '#ef4444', '#6b7280', '#c9972a', '99,102,241'].filter(
      (hex) => sheet.includes(hex)
    )
    expect(strays).toEqual([])
  })
})
