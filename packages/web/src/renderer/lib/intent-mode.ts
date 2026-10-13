/**
 * Intent resolution for the shell card's input bar.
 *
 * The bar accepts two different things: a shell command, which is written to
 * the pty, and a natural-language prompt, which starts an agent session. This
 * module decides which one the user typed.
 *
 * The rule is deliberately blunt and stated in one place, because pressing
 * return in the wrong mode either runs the wrong thing or launches an agent
 * the user did not ask for. Predictability beats cleverness — the bar shows
 * the resolved mode before submission, and the user can override it.
 */

export type IntentMode = 'shell' | 'prompt'

/**
 * Shell builtins are not files on PATH, so they never appear in the
 * executable list and would otherwise be misread as prose.
 */
export const SHELL_BUILTINS: readonly string[] = [
  'cd',
  'export',
  'alias',
  'unalias',
  'source',
  '.',
  ':',
  'unset',
  'set',
  'echo',
  'printf',
  'pushd',
  'popd',
  'dirs',
  'exit',
  'jobs',
  'fg',
  'bg',
  'kill',
  'history',
  'type',
  'command',
  'builtin',
  'eval',
  'exec',
  'read',
  'local',
  'declare',
  'typeset',
  'hash',
  'umask',
  'trap',
  'wait',
  'shift',
  'test',
  '[',
  'let',
  'return',
  'times',
  'ulimit',
  'disown',
  'suspend'
]

/** Prefixes that make the first token a path or expansion, never prose. */
const PATH_PREFIXES = ['./', '../', '/', '~', '$', '!']

/**
 * Operators that only appear in shell syntax. Checked outside quotes so a
 * prompt like `explain the "a || b" idiom` is not misread as a command.
 */
const OPERATOR = /(\|\||&&|[|;<>]|\d?>>?)/

function stripQuoted(line: string): string {
  return line.replace(/'[^']*'/g, '').replace(/"[^"]*"/g, '')
}

function hasShellSyntax(line: string, firstToken: string): boolean {
  if (PATH_PREFIXES.some((p) => firstToken.startsWith(p))) return true
  // Env assignment: FOO=bar cmd
  if (/^[A-Za-z_][A-Za-z0-9_]*=/.test(firstToken)) return true
  const unquoted = stripQuoted(line)
  if (unquoted.includes('$(') || unquoted.includes('`')) return true
  return OPERATOR.test(unquoted)
}

/**
 * Decide whether `value` is a shell command or a prompt.
 *
 * `known` is the set of runnable names — PATH executables, builtins, and any
 * command with a completion outline. An empty set means we could not find
 * out, in which case everything stays shell: guessing "prompt" would launch
 * agents on ordinary typos.
 */
export function resolveIntentMode(value: string, known: ReadonlySet<string>): IntentMode {
  const line = value.split('\n')[0].trim()
  if (!line) return 'shell'

  // Safety valve. listShellExecutables is absent in older preloads and in
  // tests; without this every multi-word input would classify as a prompt.
  if (known.size === 0) return 'shell'

  const tokens = line.split(/\s+/)
  const first = tokens[0]

  if (hasShellSyntax(line, first)) return 'shell'

  // A lone unknown word is a mistyped command, never a prompt.
  if (tokens.length === 1) return 'shell'

  // Known command first wins outright, even for `git how do I rebase`. The
  // override exists for exactly that case.
  if (known.has(first)) return 'shell'

  return 'prompt'
}
