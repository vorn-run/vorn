import fs from 'node:fs'
import path from 'node:path'

/** Where electron-log puts the app's `main.log` for a given home directory. */
export function appLogFiles(home: string, env: NodeJS.ProcessEnv = process.env): string[] {
  const files = ['Vorn', 'vorn'].map((name) => {
    if (process.platform === 'darwin') return path.join(home, 'Library', 'Logs', name, 'main.log')
    const base =
      process.platform === 'win32'
        ? (env.APPDATA ?? path.join(home, 'AppData', 'Roaming'))
        : (env.XDG_CONFIG_HOME ?? path.join(home, '.config'))
    return path.join(base, name, 'logs', 'main.log')
  })
  return [...new Set(files.map((file) => (fs.existsSync(file) ? fs.realpathSync(file) : file)))]
}

export type LogSnapshot = Map<string, number>

export function snapshotLogs(files: string[]): LogSnapshot {
  return new Map(files.map((file) => [file, fs.existsSync(file) ? fs.statSync(file).size : 0]))
}

/** Lines appended since the snapshot that mention `marker`; a rotated file is read from the start. */
export function linesSince(snapshot: LogSnapshot, marker: string): string[] {
  const found: string[] = []
  for (const [file, before] of snapshot) {
    if (!fs.existsSync(file)) continue
    const size = fs.statSync(file).size
    const start = size < before ? 0 : before
    if (size === start) continue
    const fd = fs.openSync(file, 'r')
    try {
      const buffer = Buffer.alloc(size - start)
      fs.readSync(fd, buffer, 0, buffer.length, start)
      for (const line of buffer.toString('utf8').split('\n')) {
        if (line.includes(marker)) found.push(`${file}: ${line.trim()}`)
      }
    } finally {
      fs.closeSync(fd)
    }
  }
  return found
}
