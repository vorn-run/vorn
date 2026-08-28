import type { AiAgentType } from '@vornrun/shared/types'

/**
 * Reading a launch line well enough to remove a session selector from it, and
 * refusing to when that cannot be done safely.
 *
 * ## Why the line and not the arguments
 *
 * A configured command is a string, and `buildAgentLaunchLine` interpolates it
 * unescaped -- `npx -y @anthropic-ai/claude-code` is a working configuration, and
 * the settings form splits its argument field on whitespace. So the executable is
 * not `args[0]` and there is no array to inspect. What exists is a line, and it
 * has to be read as one.
 *
 * ## Why refusing matters more than parsing
 *
 * The point of this is to leave exactly one selector on the line. Getting that
 * wrong in the cautious direction ships two, which starts the wrong session;
 * getting it wrong in the confident direction mangles somebody's command, which
 * starts nothing and loses their configuration. So anything this cannot reason
 * about is handed back untouched: an operator, an unterminated quote, a
 * substitution. `foo && claude --resume x` must never have its `claude` read as
 * an argument of `foo`, and the way to guarantee that is to decline the whole
 * line rather than to be clever about it.
 *
 * ## What is not stripped, on purpose
 *
 * `-r<id>` joined into one token, and clustered shorts like `-ir`. A leading dash
 * on the next token is ambiguous with another option's dash-leading value, and
 * a cluster cannot be split without knowing which letters take arguments. Both
 * are left alone, so a person who wrote one keeps it and ours is appended
 * beside it. Two selectors is a worse launch; a rewritten command line is a
 * worse day.
 */

export interface Token {
  /** As it appeared, quotes and all. */
  raw: string
  /** With quoting removed, which is what the shell would pass along. */
  value: string
  /** Byte offsets into the line, so a splice can put back everything else. */
  start: number
  end: number
}

/** Anything that makes a line more than one simple command. */
const REFUSED = new Set(['|', '&', ';', '<', '>', '(', ')', '\n'])

/**
 * Split a line into tokens, or answer null.
 *
 * Null is not a failure to be worked around -- it is the answer for every line
 * this must not touch.
 */
export function tokenize(line: string): Token[] | null {
  const tokens: Token[] = []
  let i = 0

  while (i < line.length) {
    while (i < line.length && (line[i] === ' ' || line[i] === '\t')) i++
    if (i >= line.length) break

    const start = i
    let value = ''

    while (i < line.length) {
      const ch = line[i]!
      if (ch === ' ' || ch === '\t') break
      if (REFUSED.has(ch)) return null
      if (ch === '`') return null
      if (ch === '$' && line[i + 1] === '(') return null

      if (ch === "'") {
        const close = line.indexOf("'", i + 1)
        if (close === -1) return null
        value += line.slice(i + 1, close)
        i = close + 1
        continue
      }

      if (ch === '"') {
        i++
        let closed = false
        while (i < line.length) {
          const c = line[i]!
          if (c === '"') {
            i++
            closed = true
            break
          }
          if (c === '`') return null
          if (c === '$' && line[i + 1] === '(') return null
          if (c === '\\' && i + 1 < line.length) {
            const next = line[i + 1]!
            if (next === '"' || next === '\\' || next === '$' || next === '`') {
              value += next
              i += 2
              continue
            }
          }
          value += c
          i++
        }
        if (!closed) return null
        continue
      }

      if (ch === '\\') {
        // A trailing backslash is a line continuation, which means this is not
        // the whole command.
        if (i + 1 >= line.length) return null
        value += line[i + 1]
        i += 2
        continue
      }

      value += ch
      i++
    }

    tokens.push({ raw: line.slice(start, i), value, start, end: i })
  }

  return tokens
}

export interface Span {
  start: number
  end: number
}

/** Whether a token could be a flag's value rather than the next flag. */
function isValue(token: Token | undefined): boolean {
  return token !== undefined && !token.value.startsWith('-')
}

/**
 * A flag and, when it has one, the token carrying its value.
 *
 * `--flag=value` is one span. `--flag value` is two, but only when the next
 * token does not itself begin with a dash -- a bare `--resume` is the
 * interactive picker, which is still a selector, and swallowing the flag after
 * it would remove something else.
 */
function flagSpans(tokens: Token[], at: number, names: string[]): Span[] | null {
  const token = tokens[at]!
  for (const name of names) {
    if (token.value === name) {
      const next = tokens[at + 1]
      return isValue(next) ? [span(token), span(next!)] : [span(token)]
    }
    if (token.value.startsWith(`${name}=`)) return [span(token)]
  }
  return null
}

const span = (t: Token): Span => ({ start: t.start, end: t.end })

/** The flag shapes that can be proven to select a session, per agent. */
const SELECTORS: Record<string, string[]> = {
  claude: ['--resume', '-r', '--continue', '-c', '--session-id'],
  copilot: ['--resume', '--session-id'],
  opencode: ['--session', '-s']
}

/**
 * Spans on the line that select a session, or null when none can be proven.
 *
 * `argsFrom` is where the configured command ends. `buildAgentLaunchLine`
 * composed this line, so it knows that offset exactly rather than having to
 * guess which token was the executable -- which is what makes
 * `/usr/local/bin/tools/claude` safe to have as a command and an argument merely
 * ending in `/claude` safe to leave alone.
 */
export function findSelectorSpans(
  line: string,
  agentType: AiAgentType,
  argsFrom: number
): Span[] | null {
  const tokens = tokenize(line)
  if (!tokens) return null

  const args = tokens.filter((t) => t.start >= argsFrom)
  const found: Span[] = []

  if (agentType === 'codex') {
    // A subcommand rather than a flag: `codex resume <id>` or `codex resume
    // --last`. Only in the first argument position, so a `resume` appearing as a
    // value further along is not one.
    const first = args[0]
    if (first?.value === 'resume') {
      found.push(span(first))
      const next = args[1]
      if (next && (next.value === '--last' || !next.value.startsWith('-'))) found.push(span(next))
    }
    return found.length ? found : null
  }

  const names = SELECTORS[agentType]
  if (!names) return null

  for (let at = 0; at < args.length; at++) {
    const spans = flagSpans(args, at, names)
    if (!spans) continue
    found.push(...spans)
    at += spans.length - 1
  }

  return found.length ? found : null
}

/**
 * The line with its session selectors removed, or unchanged when none could be
 * proven.
 *
 * Rebuilt by copying the gaps out of the original rather than by re-joining
 * tokens, so every byte outside a proven span survives exactly as it was --
 * quoting style, spacing, `$VAR`, all of it.
 */
export function stripSessionSelectors(
  line: string,
  agentType: AiAgentType,
  argsFrom: number
): string {
  const spans = findSelectorSpans(line, agentType, argsFrom)
  if (!spans || spans.length === 0) return line

  const ordered = [...spans].sort((a, b) => a.start - b.start)
  let out = ''
  let at = 0
  for (const one of ordered) {
    // Take the whitespace in front of it too, or removing a flag from the middle
    // leaves a double space and removing the last one leaves a trailing space.
    let from = one.start
    while (from > at && (line[from - 1] === ' ' || line[from - 1] === '\t')) from--
    out += line.slice(at, from)
    at = one.end
  }
  out += line.slice(at)
  return out
}
