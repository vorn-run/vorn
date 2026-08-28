import { describe, it, expect, beforeEach, vi } from 'vitest'
import type { Terminal } from '@xterm/xterm'
import {
  attachCommandBlocks,
  CommandBlockTracker,
  formatDuration,
  hasShellIntegration,
  onCommandBlocksChange,
  shortenCwd,
  type CommandBlock,
  type MarkerLike,
  type TrackerHost
} from '../src/renderer/lib/command-blocks'
import { captureBlock, clearBlockLog, getBlockLog } from '../src/renderer/lib/block-log'
import { markSeededFromServer, setDomBlockRendering } from '../src/renderer/lib/command-blocks'
import { registerBlockLogView } from '../src/renderer/lib/block-log'

class FakeMarker implements MarkerLike {
  isDisposed = false
  private disposeCbs: Array<() => void> = []
  constructor(public line: number) {}
  dispose(): void {
    if (this.isDisposed) return
    this.isDisposed = true
    this.disposeCbs.forEach((cb) => cb())
  }
  onDispose(cb: () => void): void {
    this.disposeCbs.push(cb)
  }
}

function b64(text: string): string {
  return Buffer.from(text, 'utf-8').toString('base64')
}

describe('CommandBlockTracker', () => {
  let tracker: CommandBlockTracker
  let finished: CommandBlock[]
  let now: number
  let alternate: boolean
  let nextLine: number

  beforeEach(() => {
    finished = []
    now = 1000
    alternate = false
    nextLine = 0
    const host: TrackerHost = {
      registerMarker: () => new FakeMarker(nextLine++),
      onBlockFinished: (block) => finished.push(block),
      isAlternateBuffer: () => alternate,
      now: () => now
    }
    tracker = new CommandBlockTracker(host)
  })

  function runCommand(text: string, exitCode: number, durationMs: number): void {
    tracker.handleSequence('A')
    tracker.handleCommandText(`cmd;${b64(text)}`)
    tracker.handleSequence('C')
    now += durationMs
    tracker.handleSequence(`D;${exitCode}`)
  }

  it('records a finished command with text, exit code, and duration', () => {
    runCommand('git status', 0, 420)
    expect(finished).toHaveLength(1)
    expect(finished[0].command).toBe('git status')
    expect(finished[0].exitCode).toBe(0)
    expect(finished[0].durationMs).toBe(420)
    expect(tracker.blocks).toHaveLength(1)
  })

  it('records failing exit codes', () => {
    runCommand('false', 1, 10)
    expect(finished[0].exitCode).toBe(1)
  })

  it('anchors the block to the prompt line, not the output line', () => {
    tracker.handleSequence('A') // prompt at line 0
    tracker.handleSequence('C')
    tracker.handleSequence('D;0')
    expect(tracker.blocks[0].marker.line).toBe(0)
  })

  it('ignores D without a preceding C', () => {
    tracker.handleSequence('D;0')
    expect(finished).toHaveLength(0)
  })

  it('drops the prompt marker for empty prompts (Enter on empty line)', () => {
    tracker.handleSequence('A')
    const first = tracker.jumpLines()
    tracker.handleSequence('A')
    expect(first).toHaveLength(1)
    // old marker disposed, replaced by the new prompt's marker
    expect(tracker.jumpLines()).toHaveLength(1)
  })

  it('handles a command without command text (base64 unavailable)', () => {
    tracker.handleSequence('A')
    tracker.handleSequence('C')
    tracker.handleSequence('D;0')
    expect(finished[0].command).toBeNull()
  })

  it('decodes multiline and non-ascii command text', () => {
    runCommand('echo "línea uno\nlínea dos"', 0, 5)
    expect(finished[0].command).toBe('echo "línea uno\nlínea dos"')
  })

  it('ignores sequences while the alternate buffer is active', () => {
    alternate = true
    runCommand('vim', 0, 5)
    expect(finished).toHaveLength(0)
    expect(tracker.blocks).toHaveLength(0)
  })

  it('command text does not leak into the next command', () => {
    runCommand('git status', 0, 10)
    tracker.handleSequence('A')
    tracker.handleSequence('C') // no cmd OSC this time
    tracker.handleSequence('D;0')
    expect(finished[1].command).toBeNull()
  })

  it('removes blocks whose markers are disposed (scrollback trim)', () => {
    runCommand('one', 0, 5)
    runCommand('two', 0, 5)
    expect(tracker.blocks).toHaveLength(2)
    tracker.blocks[0].marker.dispose()
    expect(tracker.blocks).toHaveLength(1)
    expect(tracker.blocks[0].command).toBe('two')
  })

  it('caps stored blocks and prunes the oldest', () => {
    for (let i = 0; i < 210; i++) runCommand(`cmd ${i}`, 0, 1)
    expect(tracker.blocks.length).toBe(200)
    expect(tracker.blocks[0].command).toBe('cmd 10')
  })

  it('jumpLines includes finished blocks and the current prompt, sorted', () => {
    runCommand('one', 0, 5) // prompt line 0
    runCommand('two', 0, 5) // prompt line 1
    tracker.handleSequence('A') // current prompt line 2
    expect(tracker.jumpLines()).toEqual([0, 1, 2])
  })

  it('malformed base64 clears the pending command instead of throwing', () => {
    tracker.handleSequence('A')
    tracker.handleCommandText('cmd;%%%not-base64%%%')
    tracker.handleSequence('C')
    tracker.handleSequence('D;0')
    expect(finished[0].command).toBeNull()
  })
})

describe('CommandBlockTracker.inputState', () => {
  let tracker: CommandBlockTracker
  let alternate: boolean

  beforeEach(() => {
    alternate = false
    let line = 0
    tracker = new CommandBlockTracker({
      registerMarker: () => new FakeMarker(line++),
      onBlockFinished: () => {},
      isAlternateBuffer: () => alternate,
      now: () => 0
    })
  })

  it('is unknown before any prompt marker arrives (no shell integration)', () => {
    expect(tracker.inputState()).toBe('unknown')
  })

  it('is prompt at the prompt and running during a command', () => {
    tracker.handleSequence('A')
    expect(tracker.inputState()).toBe('prompt')
    tracker.handleSequence('C')
    expect(tracker.inputState()).toBe('running')
    tracker.handleSequence('D;0')
    tracker.handleSequence('A')
    expect(tracker.inputState()).toBe('prompt')
  })

  it('is altScreen while the alternate buffer is active', () => {
    tracker.handleSequence('A')
    alternate = true
    expect(tracker.inputState()).toBe('altScreen')
    alternate = false
    expect(tracker.inputState()).toBe('prompt')
  })

  it('is altScreen even with no shell integration at all', () => {
    // The alternate buffer is the one answer that does not depend on the shell
    // reporting anything, and a TUI opened from an uninstrumented shell still
    // needs the terminal to have the keys.
    alternate = true
    expect(tracker.inputState()).toBe('altScreen')
  })
})

describe('output measurement', () => {
  it('records the rows a command’s output occupied', () => {
    let line = 0
    const finished: CommandBlock[] = []
    const tracker = new CommandBlockTracker({
      registerMarker: () => new FakeMarker(line),
      onBlockFinished: (block) => finished.push(block),
      isAlternateBuffer: () => false,
      now: () => 1000,
      currentLine: () => line
    })
    tracker.handleSequence('A')
    tracker.handleSequence('C')
    line = 12 // twelve rows of output scrolled past
    tracker.handleSequence('D;0')
    expect(finished[0].outputLines).toBe(12)
  })

  it('reports zero when the host cannot measure', () => {
    // Hosts without currentLine (older callers, tests) must still produce a
    // usable block rather than NaN.
    const finished: CommandBlock[] = []
    const tracker = new CommandBlockTracker({
      registerMarker: () => new FakeMarker(0),
      onBlockFinished: (block) => finished.push(block),
      isAlternateBuffer: () => false,
      now: () => 1000
    })
    tracker.handleSequence('A')
    tracker.handleSequence('C')
    tracker.handleSequence('D;0')
    expect(finished[0].outputLines).toBe(0)
  })
})

describe('block pruning', () => {
  it('notifies the host when scrollback disposes a marker', () => {
    let pruned = 0
    const markers: FakeMarker[] = []
    let line = 0
    const tracker = new CommandBlockTracker({
      registerMarker: () => {
        const m = new FakeMarker(line++)
        markers.push(m)
        return m
      },
      onBlockFinished: () => {},
      onBlocksPruned: () => pruned++,
      isAlternateBuffer: () => false,
      now: () => 1000
    })
    tracker.handleSequence('A')
    tracker.handleSequence('C')
    tracker.handleSequence('D;0')
    expect(tracker.blocks).toHaveLength(1)
    markers[0].dispose()
    expect(tracker.blocks).toHaveLength(0)
    expect(pruned).toBe(1)
  })
})

describe('runningBlock', () => {
  let tracker: CommandBlockTracker

  beforeEach(() => {
    let line = 0
    tracker = new CommandBlockTracker({
      registerMarker: () => new FakeMarker(line++),
      onBlockFinished: () => {},
      isAlternateBuffer: () => false,
      now: () => 1000
    })
  })

  it('exposes the in-flight command and clears once it finishes', () => {
    tracker.handleSequence('A')
    tracker.handleCommandText(`cmd;${b64('yarn build')}`)
    tracker.handleSequence('C')
    expect(tracker.runningBlock()?.command).toBe('yarn build')
    tracker.handleSequence('D;0')
    expect(tracker.runningBlock()).toBeNull()
  })

  it('is null while sitting at the prompt', () => {
    tracker.handleSequence('A')
    expect(tracker.runningBlock()).toBeNull()
  })
})

describe('formatDuration', () => {
  it('formats sub-second, seconds, and minutes', () => {
    expect(formatDuration(80)).toBe('0.1s')
    expect(formatDuration(950)).toBe('1s')
    expect(formatDuration(1234)).toBe('1.2s')
    expect(formatDuration(9800)).toBe('9.8s')
    expect(formatDuration(42_000)).toBe('42s')
    expect(formatDuration(83_000)).toBe('1m 23s')
  })
})

describe('attachCommandBlocks', () => {
  interface FakeTerm {
    registerDecoration: ReturnType<typeof vi.fn>
    handlers: Map<number, (data: string) => boolean>
    csiHandlers: Map<string, (params: number[]) => boolean>
    bufferListeners: Set<() => void>
  }

  function fakeTerminal(): { term: FakeTerm; asTerminal: Terminal } {
    const handlers = new Map<number, (data: string) => boolean>()
    const csiHandlers = new Map<string, (params: number[]) => boolean>()
    const bufferListeners = new Set<() => void>()
    const term = {
      registerDecoration: vi.fn(() => undefined),
      handlers,
      csiHandlers,
      bufferListeners,
      options: { theme: { background: '#141416', cursor: '#d4d4d8' } },
      cols: 80,
      buffer: {
        active: { type: 'normal', baseY: 0, cursorY: 0 },
        // Entering or leaving the alternate screen carries no OSC, so this is
        // the only thing that can announce a full-screen program opening.
        onBufferChange: (cb: () => void) => {
          bufferListeners.add(cb)
          return { dispose: () => bufferListeners.delete(cb) }
        }
      },
      registerMarker: (offset: number) => new FakeMarker(offset),
      parser: {
        registerOscHandler: (id: number, cb: (data: string) => boolean) => {
          handlers.set(id, cb)
          return { dispose: () => handlers.delete(id) }
        },
        registerCsiHandler: (id: { final: string }, cb: (params: number[]) => boolean) => {
          csiHandlers.set(id.final, cb)
          return { dispose: () => csiHandlers.delete(id.final) }
        }
      }
    }
    return { term: term as unknown as FakeTerm, asTerminal: term as unknown as Terminal }
  }

  it('draws only the duration and the closing rule, and only once done', () => {
    // Nothing is painted until a command finishes. The command's heading role
    // comes from the shell rendering it bold and the dim directory line above
    // it — not from a background band, which reads as a selection.
    const { term, asTerminal } = fakeTerminal()
    const dispose = attachCommandBlocks('t-1', asTerminal)
    const osc133 = term.handlers.get(133)!

    osc133('A')
    osc133('C')
    expect(term.registerDecoration).not.toHaveBeenCalled()

    osc133('D;0')
    expect(term.registerDecoration).toHaveBeenCalledTimes(2)
    // Both span the row. The meta is pushed right with flexbox rather than
    // the decoration's own right anchor, which lands at column 0 — on top of
    // the command it annotates.
    expect(term.registerDecoration).toHaveBeenCalledWith({ marker: expect.anything(), width: 80 })
    // The rule that closes the block.
    expect(term.registerDecoration).toHaveBeenCalledWith(
      expect.objectContaining({ width: 80, layer: 'bottom' })
    )
    dispose()
  })

  it('marks each executed command', () => {
    const { term, asTerminal } = fakeTerminal()
    const dispose = attachCommandBlocks('t-4', asTerminal)
    const osc133 = term.handlers.get(133)!
    osc133('A')
    osc133('C')
    osc133('D;0')
    osc133('A')
    osc133('C')
    osc133('D;0')
    // Two durations, two closing rules.
    expect(term.registerDecoration).toHaveBeenCalledTimes(4)
    dispose()
  })

  it('draws nothing for a prompt that never ran a command', () => {
    // Enter on an empty prompt produces no command and so no block.
    const { term, asTerminal } = fakeTerminal()
    const dispose = attachCommandBlocks('t-5', asTerminal)
    const osc133 = term.handlers.get(133)!
    osc133('A')
    osc133('A')
    expect(term.registerDecoration).not.toHaveBeenCalled()
    dispose()
  })

  it('notifies listeners on prompt, execution and completion', () => {
    const { term, asTerminal } = fakeTerminal()
    const dispose = attachCommandBlocks('t-2', asTerminal)
    let calls = 0
    const unsubscribe = onCommandBlocksChange('t-2', () => calls++)
    const osc133 = term.handlers.get(133)!
    osc133('A')
    osc133('C')
    osc133('D;0')
    expect(calls).toBeGreaterThanOrEqual(3)
    unsubscribe()
    const before = calls
    osc133('A')
    expect(calls).toBe(before)
    dispose()
  })

  it('stops notifying once the terminal is torn down', () => {
    const { term, asTerminal } = fakeTerminal()
    const dispose = attachCommandBlocks('t-3', asTerminal)
    let calls = 0
    onCommandBlocksChange('t-3', () => calls++)
    dispose()
    const osc133 = term.handlers.get(133)
    expect(osc133).toBeUndefined()
    expect(calls).toBe(0)
  })
})

describe('shortenCwd', () => {
  it('keeps the last two segments', () => {
    expect(shortenCwd('/Users/j/dev/vorn')).toBe('dev/vorn')
    expect(shortenCwd('/Users/j/dev/vorn/packages/server')).toBe('packages/server')
  })

  it('keeps short paths whole', () => {
    expect(shortenCwd('/tmp')).toBe('/tmp')
    expect(shortenCwd('/usr/local')).toBe('/usr/local')
    expect(shortenCwd('/')).toBe('/')
  })

  it('returns null when the shell never reported one', () => {
    expect(shortenCwd(null)).toBeNull()
  })
})

describe('block cwd', () => {
  it('records the directory reported at the prompt before the command', () => {
    let line = 0
    const finished: CommandBlock[] = []
    const tracker = new CommandBlockTracker({
      registerMarker: () => new FakeMarker(line++),
      onBlockFinished: (b) => finished.push(b),
      isAlternateBuffer: () => false,
      now: () => 1000
    })
    tracker.handleSequence('A')
    tracker.handleCommandText('cwd;/Users/j/dev/vorn')
    tracker.handleSequence('C')
    tracker.handleSequence('D;0')
    expect(finished[0].cwd).toBe('/Users/j/dev/vorn')

    // A `cd` reports the new directory at the next prompt, and the command
    // that follows belongs to it.
    tracker.handleSequence('A')
    tracker.handleCommandText('cwd;/Users/j/dev/vorn/docs')
    tracker.handleSequence('C')
    tracker.handleSequence('D;0')
    expect(finished[1].cwd).toBe('/Users/j/dev/vorn/docs')
  })
})

describe('clear', () => {
  /**
   * `clear` emits CSI 3 J (erase scrollback) before clearing the screen.
   * Finished commands are lifted out of the buffer into the log, so the log is
   * the scrollback — if the sequence does not reach it, `clear` visibly does
   * nothing.
   */
  function attachWithLog(): {
    csiHandlers: Map<string, (params: number[]) => boolean>
  } {
    const handlers = new Map<number, (data: string) => boolean>()
    const csiHandlers = new Map<string, (params: number[]) => boolean>()
    const term = {
      registerDecoration: vi.fn(() => undefined),
      options: { theme: {} },
      cols: 80,
      clear: vi.fn(),
      buffer: {
        active: { type: 'normal', baseY: 0, cursorY: 0, length: 1 },
        onBufferChange: () => ({ dispose: () => {} })
      },
      registerMarker: (offset: number) => new FakeMarker(offset),
      parser: {
        registerOscHandler: (id: number, cb: (data: string) => boolean) => {
          handlers.set(id, cb)
          return { dispose: () => handlers.delete(id) }
        },
        registerCsiHandler: (id: { final: string }, cb: (params: number[]) => boolean) => {
          csiHandlers.set(id.final, cb)
          return { dispose: () => csiHandlers.delete(id.final) }
        }
      }
    }
    attachCommandBlocks('clear-term', term as unknown as Terminal)
    return { csiHandlers }
  }

  beforeEach(() => {
    clearBlockLog('clear-term')
  })

  function seedBlock(): void {
    captureBlock({
      terminalId: 'clear-term',
      buffer: { length: 1, getLine: () => undefined } as never,
      startLine: 0,
      endLine: 0,
      command: 'ls',
      exitCode: 0,
      durationMs: 10,
      cwd: null
    })
  }

  it('erases the log so the command has a visible effect', () => {
    const { csiHandlers } = attachWithLog()
    seedBlock()
    expect(getBlockLog('clear-term')).toHaveLength(1)

    csiHandlers.get('J')?.([3])
    expect(getBlockLog('clear-term')).toHaveLength(0)
  })

  it('leaves the log alone on a plain screen erase', () => {
    // Full-screen programs send CSI 2 J to repaint. Treating that as "discard
    // history" would wipe the log every time one redrew.
    const { csiHandlers } = attachWithLog()
    seedBlock()

    csiHandlers.get('J')?.([2])
    expect(getBlockLog('clear-term')).toHaveLength(1)
  })

  it('lets xterm run its own erase handler', () => {
    const { csiHandlers } = attachWithLog()
    // Returning true would swallow the sequence and the screen would never
    // clear.
    expect(csiHandlers.get('J')?.([3])).toBe(false)
  })
})

describe('shell integration detection', () => {
  /**
   * The shim is only installed for zsh. Every other shell — bash, fish,
   * PowerShell, cmd — runs without it, and the marker itself is the only
   * evidence of which kind this is.
   */
  function fake(): { term: Terminal; handlers: Map<number, (d: string) => boolean> } {
    const handlers = new Map<number, (d: string) => boolean>()
    const bufferListeners = new Set<() => void>()
    const term = {
      registerDecoration: vi.fn(() => undefined),
      options: { theme: {} },
      cols: 80,
      clear: vi.fn(),
      buffer: {
        active: { type: 'normal', baseY: 0, cursorY: 0, length: 1 },
        onBufferChange: (cb: () => void) => {
          bufferListeners.add(cb)
          return { dispose: () => bufferListeners.delete(cb) }
        }
      },
      registerMarker: (offset: number) => new FakeMarker(offset),
      parser: {
        registerOscHandler: (id: number, cb: (d: string) => boolean) => {
          handlers.set(id, cb)
          return { dispose: () => handlers.delete(id) }
        },
        registerCsiHandler: () => ({ dispose: () => {} })
      }
    }
    return { term: term as unknown as Terminal, handlers, bufferListeners }
  }

  it('reports nothing until a boundary actually arrives', () => {
    const { term } = fake()
    attachCommandBlocks('unmarked', term)
    expect(hasShellIntegration('unmarked')).toBe(false)
  })

  it('announces an alternate-buffer switch, which carries no OSC', () => {
    // A program can take the whole screen without the shell saying anything, so
    // this is the only thing that can tell a listener vim opened.
    const { term, bufferListeners } = fake()
    attachCommandBlocks('tui', term)
    let notified = 0
    onCommandBlocksChange('tui', () => notified++)
    expect(bufferListeners.size).toBe(1)
    bufferListeners.forEach((cb) => cb())
    expect(notified).toBe(1)
  })

  it('stops listening to the buffer when torn down', () => {
    const { term, bufferListeners } = fake()
    const dispose = attachCommandBlocks('tui-gone', term)
    expect(bufferListeners.size).toBe(1)
    dispose()
    expect(bufferListeners.size).toBe(0)
  })

  it('reports integration from the first prompt marker', () => {
    const { term, handlers } = fake()
    attachCommandBlocks('marked', term)
    handlers.get(133)?.('A')
    expect(hasShellIntegration('marked')).toBe(true)
  })

  it('forgets the terminal when it is torn down', () => {
    const { term, handlers } = fake()
    const dispose = attachCommandBlocks('gone', term)
    handlers.get(133)?.('A')
    dispose()
    expect(hasShellIntegration('gone')).toBe(false)
  })
})

describe('shells that cannot report execution start', () => {
  /**
   * PowerShell's prompt function runs between commands and cmd.exe can only
   * decorate PROMPT, so neither emits C. Everything arrives at the next prompt:
   * the command text, how long it took, then D.
   */
  function tracker(finished: CommandBlock[]): CommandBlockTracker {
    let line = 0
    return new CommandBlockTracker({
      registerMarker: () => new FakeMarker(line++),
      onBlockFinished: (b) => finished.push(b),
      isAlternateBuffer: () => false,
      now: () => 1000
    })
  }

  it('still produces a block when only A and D arrive', () => {
    const finished: CommandBlock[] = []
    const t = tracker(finished)
    t.handleSequence('A')
    t.handleCommandText(`cmd;${btoa('Get-ChildItem')}`)
    t.handleSequence('D;0')
    expect(finished).toHaveLength(1)
    expect(finished[0].command).toBe('Get-ChildItem')
  })

  it('keeps the exit code reported alongside the missing marker', () => {
    const finished: CommandBlock[] = []
    const t = tracker(finished)
    t.handleSequence('A')
    t.handleSequence('D;127')
    expect(finished[0].exitCode).toBe(127)
  })

  it('uses the reported duration rather than measuring from the marker', () => {
    // Measured here it would be zero, because the block starts and ends in the
    // same instant — every command would claim to be instant.
    const finished: CommandBlock[] = []
    const t = tracker(finished)
    t.handleSequence('A')
    t.handleCommandText('dur;2500')
    t.handleSequence('D;0')
    expect(finished[0].durationMs).toBe(2500)
  })

  it('does not carry a duration over to the next command', () => {
    const finished: CommandBlock[] = []
    const t = tracker(finished)
    t.handleSequence('A')
    t.handleCommandText('dur;2500')
    t.handleSequence('D;0')
    t.handleSequence('A')
    t.handleSequence('D;0')
    expect(finished[1].durationMs).toBe(0)
  })

  it('still prefers C when the shell does emit it', () => {
    const finished: CommandBlock[] = []
    const t = tracker(finished)
    t.handleSequence('A')
    t.handleSequence('C')
    t.handleSequence('D;0')
    expect(finished).toHaveLength(1)
  })
})

describe('working directory reporting', () => {
  function trackerWithCwd(): { t: CommandBlockTracker; finished: CommandBlock[] } {
    let line = 0
    const finished: CommandBlock[] = []
    const t = new CommandBlockTracker({
      registerMarker: () => new FakeMarker(line++),
      onBlockFinished: (b) => finished.push(b),
      isAlternateBuffer: () => false,
      now: () => 1000
    })
    return { t, finished }
  }

  it('accepts a Windows drive path', () => {
    // Requiring a leading slash silently dropped every directory on Windows.
    const { t, finished } = trackerWithCwd()
    t.handleSequence('A')
    t.handleCommandText('cwd;C:\\Users\\j\\dev')
    t.handleSequence('C')
    t.handleSequence('D;0')
    expect(finished[0].cwd).toBe('C:\\Users\\j\\dev')
  })

  it('ignores something that is not a path at all', () => {
    const { t, finished } = trackerWithCwd()
    t.handleSequence('A')
    t.handleCommandText('cwd;not-a-path')
    t.handleSequence('C')
    t.handleSequence('D;0')
    expect(finished[0].cwd).toBeNull()
  })
})

describe('shells that carry the command in their own marker', () => {
  /**
   * fish 4 marks prompts itself and percent-encodes the command line inside
   * its C marker. That marker arrives before anything we could emit, so it is
   * the only place a fish block's title can come from.
   */
  it('reads the command line out of a fish C marker', () => {
    let line = 0
    const finished: CommandBlock[] = []
    const t = new CommandBlockTracker({
      registerMarker: () => new FakeMarker(line++),
      onBlockFinished: (b) => finished.push(b),
      isAlternateBuffer: () => false,
      now: () => 1000
    })
    t.handleSequence('A;click_events=1')
    t.handleSequence('C;cmdline_url=git%20status%20--short')
    t.handleSequence('D;0')
    expect(finished[0].command).toBe('git status --short')
  })

  it('leaves the block untitled rather than guessing at bad encoding', () => {
    let line = 0
    const finished: CommandBlock[] = []
    const t = new CommandBlockTracker({
      registerMarker: () => new FakeMarker(line++),
      onBlockFinished: (b) => finished.push(b),
      isAlternateBuffer: () => false,
      now: () => 1000
    })
    t.handleSequence('A')
    t.handleSequence('C;cmdline_url=%E0%A4%A')
    t.handleSequence('D;0')
    expect(finished[0].command).toBeNull()
  })
})

describe('a shell that also emits markers itself', () => {
  /**
   * Clink does this for cmd, as do prompt frameworks and hand-rolled
   * integrations. Duplicated markers must degrade to a correct block rather
   * than losing the command outright.
   */
  function tracker(finished: CommandBlock[]): CommandBlockTracker {
    let line = 0
    return new CommandBlockTracker({
      registerMarker: () => new FakeMarker(line++),
      onBlockFinished: (b) => finished.push(b),
      isAlternateBuffer: () => false,
      now: () => 1000
    })
  }

  it('still produces one block when every marker arrives twice', () => {
    const finished: CommandBlock[] = []
    const t = tracker(finished)
    t.handleSequence('A')
    t.handleSequence('A')
    t.handleCommandText(`cmd;${btoa('ls')}`)
    t.handleSequence('C')
    t.handleSequence('C')
    t.handleSequence('D;0')
    t.handleSequence('D;0')
    expect(finished).toHaveLength(1)
    expect(finished[0].command).toBe('ls')
  })

  it('keeps tracking the command after a repeated marker', () => {
    const finished: CommandBlock[] = []
    const t = tracker(finished)
    t.handleSequence('A')
    t.handleSequence('C')
    t.handleSequence('C')
    t.handleSequence('D;3')
    // A duplicated C used to strand the block with no start, so it never
    // finished and the next command inherited the confusion.
    t.handleSequence('A')
    t.handleCommandText(`cmd;${btoa('pwd')}`)
    t.handleSequence('C')
    t.handleSequence('D;0')
    expect(finished.map((b) => b.exitCode)).toEqual([3, 0])
    expect(finished[1].command).toBe('pwd')
  })
})

describe('a pane seeded with a screen it did not draw', () => {
  /**
   * A pane attaching to a session already running is given what that session
   * has shown so far, and that screen then lives in the terminal and nowhere
   * else -- the block log dies with the window that built it. Lifting a
   * finished command calls `term.clear()`, which drops everything above the
   * prompt, so the first command after attaching used to take the whole
   * restored screen with it: reopen, type anything, and the session's history
   * is gone.
   */
  const ID = 'seeded-term'

  function cellOf(ch: string): Record<string, () => unknown> {
    return {
      getChars: () => ch,
      getWidth: () => 1,
      getFgColor: () => 0,
      getBgColor: () => 0,
      isFgDefault: () => true,
      isBgDefault: () => true,
      isFgRGB: () => false,
      isBgRGB: () => false,
      isBold: () => 0,
      isItalic: () => 0,
      isDim: () => 0,
      isUnderline: () => 0,
      isInverse: () => 0,
      isStrikethrough: () => 0
    }
  }

  function terminalHolding(lines: string[], markerAt: number) {
    const handlers = new Map<number, (data: string) => boolean>()
    const clear = vi.fn()
    const term = {
      registerDecoration: vi.fn(() => undefined),
      options: { theme: {} },
      cols: 80,
      clear,
      buffer: {
        active: {
          type: 'normal',
          baseY: 0,
          cursorY: 0,
          length: lines.length,
          getLine: (y: number) =>
            lines[y] === undefined
              ? undefined
              : { length: lines[y].length, getCell: (x: number) => cellOf(lines[y][x] ?? ' ') }
        },
        onBufferChange: () => ({ dispose: () => {} })
      },
      registerMarker: () => new FakeMarker(markerAt),
      parser: {
        registerOscHandler: (id: number, cb: (data: string) => boolean) => {
          handlers.set(id, cb)
          return { dispose: () => handlers.delete(id) }
        },
        registerCsiHandler: () => ({ dispose: () => {} })
      }
    }
    return { handlers, clear, asTerminal: term as unknown as Terminal }
  }

  beforeEach(() => {
    clearBlockLog(ID)
    setDomBlockRendering(true)
    registerBlockLogView(ID)
  })

  /** OSC 133 carries no command text, so blocks are told apart by what is in them. */
  const holding = (needle: string): number =>
    getBlockLog(ID).filter((b) => JSON.stringify(b.rows).includes(needle)).length

  function runOneCommand(handlers: Map<number, (data: string) => boolean>): void {
    const osc133 = handlers.get(133)!
    osc133('A')
    osc133('C')
    osc133('D;0')
  }

  it('lifts the restored screen into the log before the first clear reaches it', () => {
    const { handlers, clear, asTerminal } = terminalHolding(['from before', 'the pane', '$ ls'], 2)
    attachCommandBlocks(ID, asTerminal)
    markSeededFromServer(ID)

    runOneCommand(handlers)

    // Two: what was restored, and the command just run.
    expect(getBlockLog(ID)).toHaveLength(2)
    expect(holding('from before')).toBe(1)
    expect(clear).toHaveBeenCalled()
  })

  it('does it once, not on every command after', () => {
    const { handlers, asTerminal } = terminalHolding(['from before', 'the pane', '$ ls'], 2)
    attachCommandBlocks(ID, asTerminal)
    markSeededFromServer(ID)

    runOneCommand(handlers)
    runOneCommand(handlers)

    expect(holding('from before')).toBe(1)
  })

  it('adds nothing for a pane that grew from an empty terminal', () => {
    // Never seeded, so there is nothing above the first command that anything
    // else is holding a copy of.
    const { handlers, asTerminal } = terminalHolding(['$ ls'], 0)
    attachCommandBlocks(ID, asTerminal)

    runOneCommand(handlers)

    expect(getBlockLog(ID)).toHaveLength(1)
  })
})
