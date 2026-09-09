import log from '../logger'

/** Resizing a pty is a TIOCSWINSZ ioctl, which Node has no binding for; node-pty's does. */
interface PtyBinding {
  resize(fd: number, cols: number, rows: number, pixelWidth: number, pixelHeight: number): void
}

let binding: PtyBinding | null = null
let looked = false

function load(): PtyBinding | null {
  if (looked) return binding
  looked = true
  try {
    // node-pty's own loader, because the binding's path varies by how it was installed.
    // eslint-disable-next-line @typescript-eslint/no-require-imports
    const utils = require('node-pty/lib/utils') as {
      loadNativeModule(name: string): { module: PtyBinding }
    }
    binding = utils.loadNativeModule('pty').module
  } catch (err) {
    log.error({ err }, '[handoff] node-pty binding unreachable; adopted panes cannot resize')
    binding = null
  }
  return binding
}

/** Answers rather than throws: a pane at the wrong width must not end a server holding every terminal. */
export function resizeFd(fd: number, cols: number, rows: number): boolean {
  const native = load()
  if (!native) return false
  try {
    native.resize(fd, cols, rows, 0, 0)
    return true
  } catch (err) {
    log.warn({ err, fd, cols, rows }, '[handoff] could not resize an adopted pty')
    return false
  }
}

export function canResizeAdopted(): boolean {
  return load() !== null
}
