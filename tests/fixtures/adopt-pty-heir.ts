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

const until = (match: RegExp, ms = 10_000): Promise<boolean> =>
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

  // A non-blocking master answers a full buffer with EAGAIN, which is where a paste
  // is silently truncated. Echo off first, or this measures rendering instead.
  adopted.write('stty -echo\r')
  await until(/\$|#|>/, 5_000)
  seen = ''
  adopted.write(`: ${'x'.repeat(64_000)}\r`)
  adopted.write('echo BURST_DONE\r')
  const burst = await until(/BURST_DONE\r?\n/, 20_000)

  seen = ''
  adopted.write('exit\r')
  await until(/never/, 3_000)

  if (process.env.HEIR_DEBUG) {
    process.stderr.write(`[heir] read=${read} cols=${cols} burst=${burst} exited=${exited}\n`)
    process.stderr.write(`[heir] tail=${JSON.stringify(seen.slice(-400))}\n`)
  }
  process.send?.({ read, cols, burst, exited })
  process.exit(0)
}

void main()
