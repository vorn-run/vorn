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

const until = (match: RegExp, ms = 30_000): Promise<boolean> =>
  new Promise((resolve) => {
    const started = Date.now()
    const tick = setInterval(() => {
      if (match.test(seen)) {
        clearInterval(tick)
        resolve(true)
      } else if (Date.now() - started > ms) {
        clearInterval(tick)
        resolve(false)
      }
    }, 25)
  })

async function main(): Promise<void> {
  // Readiness before anything is typed. Bracketed paste going on means readline
  // is waiting for input; the fallback covers a shell that never enables it.
  // eslint-disable-next-line no-control-regex
  if (!(await until(/\x1b\[\?2004h/, 10_000))) await until(/\S/, 10_000)

  const read = await (async () => {
    adopted.write('echo ADOPTED_READ\r')
    return until(/ADOPTED_READ\r?\n/)
  })()

  // `tput cols` asks the tty itself, so the answer proves TIOCSWINSZ landed here.
  adopted.resize(120, 40)
  seen = ''
  adopted.write('tput cols\r')
  await until(/\b120\b/)
  const cols = /(?:^|\D)(\d{2,4})\r?\n/.exec(seen)?.[1] ?? 'none'

  seen = ''
  adopted.write('exit\r')
  await until(/never/, 3_000)

  // Always, not only under a flag: this runs in a process the test does not own,
  // and a bare "expected true" tells whoever reads CI nothing at all.
  process.stderr.write(`[heir] read=${read} cols=${cols} exited=${exited}\n`)
  process.stderr.write(`[heir] tail=${JSON.stringify(seen.slice(-400))}\n`)
  process.send?.({ read, cols, exited })
  process.exit(0)
}

void main()
