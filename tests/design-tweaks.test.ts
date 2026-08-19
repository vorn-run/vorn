// @vitest-environment jsdom
import { describe, it, expect, beforeEach } from 'vitest'
import { loadTweaks, saveTweak, mergeTweaks, resetTweaks } from '../src/renderer/lib/design-tweaks'

/**
 * What survives a repaint.
 *
 * The pane reloads a design whenever its file changes, so without these the
 * loop would reset every value the moment the agent touched anything — you
 * would be punished for the agent's turn. Each case here is one where losing or
 * mistyping a value would look like the design misbehaving rather than like
 * storage doing something wrong.
 */

const FILE = '/repo/design/budget.dc.html'

beforeEach(() => resetTweaks())

describe('remembering what you set', () => {
  it('keeps a value across a reload', () => {
    saveTweak(FILE, 'plan', 9000)
    expect(loadTweaks(FILE)).toEqual({ plan: 9000 })
  })

  it('keeps files apart', () => {
    saveTweak(FILE, 'plan', 9000)
    saveTweak('/repo/design/other.dc.html', 'plan', 100)
    expect(loadTweaks(FILE)).toEqual({ plan: 9000 })
  })

  it('accumulates rather than replacing the file’s other values', () => {
    saveTweak(FILE, 'plan', 9000)
    saveTweak(FILE, 'sketchy', false)
    expect(loadTweaks(FILE)).toEqual({ plan: 9000, sketchy: false })
  })

  it('refuses a value no control could have produced', () => {
    // A control emits a string, a number or a boolean. Anything else came from
    // somewhere else, and storing it would hand the design a value it has no
    // code to render.
    saveTweak(FILE, 'obj', { nested: true })
    saveTweak(FILE, 'nan', Number.NaN)
    saveTweak(FILE, 'nothing', undefined)
    expect(loadTweaks(FILE)).toEqual({})
  })

  it('answers nothing for a file it has never seen', () => {
    expect(loadTweaks('/repo/never-opened.dc.html')).toEqual({})
  })
})

describe('surviving a store someone edited', () => {
  const write = (v: unknown): void => localStorage.setItem('vorn:designTweaks', JSON.stringify(v))

  it('reads nothing out of unparseable storage rather than throwing', () => {
    localStorage.setItem('vorn:designTweaks', '{ not json')
    expect(loadTweaks(FILE)).toEqual({})
  })

  it('drops entries of the wrong shape and keeps the rest', () => {
    write({
      [FILE]: { at: 1, values: { plan: 9000, bad: { deep: 1 } } },
      '/broken': 'not an object',
      '/empty': { at: 1, values: {} }
    })
    expect(loadTweaks(FILE)).toEqual({ plan: 9000 })
    expect(loadTweaks('/broken')).toEqual({})
    expect(loadTweaks('/empty')).toEqual({})
  })

  it('does not grow without bound', () => {
    // Keyed by absolute path, this gains an entry per design opened and the
    // renderer cannot stat a path to know a file is gone. Age is the only
    // signal there is.
    for (let i = 0; i < 260; i++) saveTweak(`/repo/d${i}.dc.html`, 'plan', i)
    const all = JSON.parse(localStorage.getItem('vorn:designTweaks') ?? '{}')
    expect(Object.keys(all).length).toBeLessThanOrEqual(200)
    // The most recent survive.
    expect(loadTweaks('/repo/d259.dc.html')).toEqual({ plan: 259 })
  })
})

describe('what a design opens with', () => {
  const declared = {
    plan: { default: 6000 },
    sketchy: { default: true },
    variance: { default: 'Both' }
  }

  it('is the declared defaults when nothing was set', () => {
    expect(mergeTweaks(declared, {})).toEqual({ plan: 6000, sketchy: true, variance: 'Both' })
  })

  it('is your value where you set one', () => {
    expect(mergeTweaks(declared, { plan: 9000 })).toEqual({
      plan: 9000,
      sketchy: true,
      variance: 'Both'
    })
  })

  it('drops a stored value for a tweak the design no longer declares', () => {
    // No control renders it and no code reads it, so carrying it would keep a
    // dead value alive forever with nothing to spend it on.
    expect(mergeTweaks(declared, { removed: 'gone' })).not.toHaveProperty('removed')
  })

  it('ignores a stored value whose type no longer matches', () => {
    // A design that changed `plan` from a number to a select would otherwise be
    // handed the old number as its selected option.
    expect(mergeTweaks(declared, { plan: 'nine thousand' }).plan).toBe(6000)
    expect(mergeTweaks(declared, { sketchy: 1 }).sketchy).toBe(true)
  })

  it('drops a selected option the design no longer offers', () => {
    // The loop's own failure mode: you pick 'Pct', the agent rewrites the design
    // without it, and on the repaint the bar would show the first option while
    // the page held the old one — two states with no event to reconcile them.
    const withOptions = { variance: { default: 'Both', options: ['Amount', 'Both'] } }
    expect(mergeTweaks(withOptions, { variance: 'Pct' }).variance).toBe('Both')
    expect(mergeTweaks(withOptions, { variance: 'Amount' }).variance).toBe('Amount')
  })

  it('takes what the page holds when you have set nothing', () => {
    // A page can arrive holding something other than its declared default —
    // its own script may have decided. With no adjustment of yours, that is
    // what is on screen and what the controls should show.
    expect(mergeTweaks(declared, {}, { plan: 7500 }).plan).toBe(7500)
  })

  it('puts your value above what the page holds', () => {
    // The precedence that matters: a freshly loaded page reports its defaults,
    // and letting those win would undo your adjustment on every repaint.
    expect(mergeTweaks(declared, { plan: 9000 }, { plan: 6000 }).plan).toBe(9000)
  })

  it('ignores a page value of the wrong type', () => {
    expect(mergeTweaks(declared, {}, { plan: 'lots' }).plan).toBe(6000)
  })

  it('has nothing to merge for a design with no tweaks', () => {
    expect(mergeTweaks(undefined, { plan: 9000 })).toEqual({})
  })
})
