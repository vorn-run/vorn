/** A replacement server reduced to the part under test: adoption is a two-process act. */
import tty from 'node:tty'
import { AdoptedPty } from '../../packages/server/src/handoff/adopted-pty'

/** Slot 3 is the IPC channel, so the first pane lands on 4. Mirrors FIRST_PTY_SLOT. */
const FD = 4

if (process.env.HEIR_DEBUG) {
  process.stderr.write(`[heir] isatty(${FD})=${tty.isatty(FD)}\n`)
}
const adopted = new AdoptedPty(FD, Number(process.env.SHELL_PID))

let seen = ''
let exited = false
adopted.onData((data) => {
  seen += data
})
adopted.onExit(() => {
  exited = true
})

const born = Date.now()
/** Resolves with the match, or undefined after `ms`, saying what was seen so a CI log explains itself. */
const until = (match: RegExp, ms = 30_000): Promise<RegExpExecArray | undefined> =>
  new Promise((resolve) => {
    const started = Date.now()
    const tick = setInterval(() => {
      const found = match.exec(seen)
      if (found) {
        clearInterval(tick)
        resolve(found)
      } else if (Date.now() - started > ms) {
        clearInterval(tick)
        process.stderr.write(
          `[heir] ${match} not seen after ${ms}ms (t+${Date.now() - born}ms); seen=${JSON.stringify(seen)}\n`
        )
        resolve(undefined)
      }
    }, 25)
  })

async function main(): Promise<void> {
  // No readiness wait: the parent handed the pty over only after the shell had
  // printed its prompt, and it kept that chunk, so nothing arrives here until typed.
  adopted.write('echo ADOPTED_READ\r')
  const read = (await until(/ADOPTED_READ\r?\n/)) !== undefined

  // `tput cols` asks the tty itself, so the answer proves TIOCSWINSZ landed here.
  adopted.resize(120, 40)
  seen = ''
  adopted.write('tput cols\r')
  const cols = (await until(/(?:^|\D)(\d{2,4})\r?\n/))?.[1] ?? 'none'

  seen = ''
  adopted.write('exit\r')
  const asked = Date.now()
  while (!exited && Date.now() - asked < 3_000) await new Promise((r) => setTimeout(r, 25))

  // Always, not only under a flag: this runs in a process the test does not own,
  // and a bare "expected true" tells whoever reads CI nothing at all.
  process.stderr.write(`[heir] read=${read} cols=${cols} exited=${exited}\n`)
  process.stderr.write(`[heir] tail=${JSON.stringify(seen.slice(-400))}\n`)
  process.send?.({ read, cols, exited })
  process.exit(0)
}

void main()
