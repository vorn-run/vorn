import { describe, it, expect } from 'vitest'
import { tokenize, stripSessionSelectors } from '../packages/server/src/launch-tokens'

/**
 * Removing a session selector from a line somebody else wrote.
 *
 * The rule is that exactly one selector reaches the agent. Being too cautious
 * ships two and starts the wrong session; being too confident rewrites somebody's
 * command and starts nothing. So most of this file is about what is *not*
 * touched, and about lines this declines to read at all.
 */

/** Where the configured command ends. The caller composed the line, so it knows. */
const from = (command: string): number => command.length

describe('reading a line', () => {
  it('keeps quoting out of the value and the offsets on the line', () => {
    const tokens = tokenize(`claude --model "gpt 4"`)
    expect(tokens?.map((t) => t.value)).toEqual(['claude', '--model', 'gpt 4'])
    expect(tokens?.map((t) => t.raw)).toEqual(['claude', '--model', '"gpt 4"'])
  })

  it.each([
    ['a pipeline', 'claude | tee log'],
    ['a conjunction', 'foo && claude --resume x'],
    ['a sequence', 'claude; echo done'],
    ['a redirect', 'claude > out.txt'],
    ['a subshell', 'claude $(cat id)'],
    ['a backtick', 'claude `cat id`'],
    ['an unterminated single quote', "claude --model 'gpt"],
    ['an unterminated double quote', 'claude --model "gpt'],
    ['a line continuation', 'claude --model \\']
  ])('refuses %s rather than guessing at it', (_label, line) => {
    // The refusal is what makes failing open safe. In `foo && claude`, reading
    // `claude` as an argument of `foo` and editing around it would produce a
    // command that does something else entirely.
    expect(tokenize(line)).toBeNull()
  })
})

describe('a selector already on the line', () => {
  it.each([
    ['a flag and its value', 'claude --resume old-id'],
    ['a joined flag', 'claude --resume=old-id'],
    ['the short form', 'claude -r old-id'],
    ['continue', 'claude --continue'],
    ['the short continue', 'claude -c'],
    ['a pinned id', 'claude --session-id old-id']
  ])('is removed: %s', (_label, line) => {
    expect(stripSessionSelectors(line, 'claude', from('claude'))).toBe('claude')
  })

  it('takes the space in front of it, so nothing is left doubled', () => {
    expect(stripSessionSelectors('claude --resume old --model x', 'claude', from('claude'))).toBe(
      'claude --model x'
    )
  })

  it('leaves a bare flag at the end of the line as just the flag', () => {
    // `--resume` with nothing after it is the interactive picker. Still a
    // selector, but the next token is not its value because there is none.
    expect(stripSessionSelectors('claude --model x --resume', 'claude', from('claude'))).toBe(
      'claude --model x'
    )
  })

  it('does not swallow the next flag as though it were a value', () => {
    expect(stripSessionSelectors('claude --resume --verbose', 'claude', from('claude'))).toBe(
      'claude --verbose'
    )
  })
})

describe('everything that is not a selector', () => {
  it('survives byte for byte, quoting and all', () => {
    const line = `claude --model 'gpt 4' --resume old --dir "$HOME/dev"`
    expect(stripSessionSelectors(line, 'claude', from('claude'))).toBe(
      `claude --model 'gpt 4' --dir "$HOME/dev"`
    )
  })

  it('includes an argument that merely looks like the executable', () => {
    // `--wrapper /usr/local/bin/claude` is a path somebody passed, not the
    // command. Nothing about it selects a session.
    const line = 'claude --wrapper /usr/local/bin/claude --resume old'
    expect(stripSessionSelectors(line, 'claude', from('claude'))).toBe(
      'claude --wrapper /usr/local/bin/claude'
    )
  })

  it('includes the command itself, however many words it is', () => {
    // The settings form splits an argument field on whitespace, so a command of
    // `npx -y @anthropic-ai/claude-code` is a working configuration.
    const command = 'npx -y @anthropic-ai/claude-code'
    expect(stripSessionSelectors(`${command} --resume old`, 'claude', from(command))).toBe(command)
  })
})

describe('a flag that takes no value', () => {
  it('does not swallow the argument after it', () => {
    // `--continue` takes no value, so what follows is somebody's prompt. An
    // earlier version treated any non-dash token as the flag's value and turned
    // this into `claude` -- not a refusal, but a command that runs and asks the
    // agent nothing. Found by differential-testing the rewrite against /bin/sh.
    expect(
      stripSessionSelectors("claude --continue 'fix the failing test'", 'claude', from('claude'))
    ).toBe("claude 'fix the failing test'")
  })

  it('does not swallow it for the short form either', () => {
    expect(stripSessionSelectors('claude -c my-prompt', 'claude', from('claude'))).toBe(
      'claude my-prompt'
    )
  })

  it('still removes a flag that does take one, with its value', () => {
    expect(stripSessionSelectors('claude --resume old-id rest', 'claude', from('claude'))).toBe(
      'claude rest'
    )
  })
})

describe('after a bare double dash', () => {
  it('nothing is a flag any more', () => {
    // `--` is how a prompt beginning with a dash is passed. Rewriting past it
    // changes what the agent is asked rather than which session it opens.
    const line = 'claude -- --resume looks-like-a-flag'
    expect(stripSessionSelectors(line, 'claude', from('claude'))).toBe(line)
  })

  it('but a selector before it is still removed', () => {
    expect(
      stripSessionSelectors('claude --resume old -- --resume text', 'claude', from('claude'))
    ).toBe('claude -- --resume text')
  })
})

describe('what is deliberately left alone', () => {
  it('keeps a joined short selector, and lets two of them ship', () => {
    // `-rold` cannot be told apart from another option taking a dash-leading
    // value. Rewriting on a guess is worse than an extra selector somebody put
    // there themselves.
    const line = 'claude -rold-id'
    expect(stripSessionSelectors(line, 'claude', from('claude'))).toBe(line)
  })

  it('keeps a cluster of short flags', () => {
    // Splitting `-ir` needs to know which letters take arguments.
    const line = 'claude -ir'
    expect(stripSessionSelectors(line, 'claude', from('claude'))).toBe(line)
  })

  it('hands back a line it refused to read', () => {
    const line = 'foo && claude --resume old'
    expect(stripSessionSelectors(line, 'claude', from('foo'))).toBe(line)
  })

  it('leaves an agent with no resume of its own untouched', () => {
    const line = 'gemini --resume old'
    expect(stripSessionSelectors(line, 'gemini', from('gemini'))).toBe(line)
  })
})

describe('per agent', () => {
  it('removes copilot flags but not claude-only ones', () => {
    expect(stripSessionSelectors('copilot --resume old -c', 'copilot', from('copilot'))).toBe(
      'copilot -c'
    )
  })

  it('removes an opencode session flag in both forms', () => {
    expect(stripSessionSelectors('opencode --session old', 'opencode', from('opencode'))).toBe(
      'opencode'
    )
    expect(stripSessionSelectors('opencode -s old', 'opencode', from('opencode'))).toBe('opencode')
  })

  it('removes a codex resume subcommand and its id', () => {
    expect(stripSessionSelectors('codex resume old-id --model o3', 'codex', from('codex'))).toBe(
      'codex --model o3'
    )
  })

  it('removes a codex resume --last', () => {
    expect(stripSessionSelectors('codex resume --last', 'codex', from('codex'))).toBe('codex')
  })

  it('leaves a codex resume that is not in subcommand position', () => {
    // `--note resume` is a value somebody passed. A subcommand is only a
    // subcommand where a subcommand can go.
    const line = 'codex --note resume'
    expect(stripSessionSelectors(line, 'codex', from('codex'))).toBe(line)
  })
})
