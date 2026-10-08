import fs from 'node:fs'
import path from 'node:path'

/**
 * Answers the TypeScript path gave, recorded so vornd's can be checked
 * against them once that path is gone (`tests/fixtures/js-reference`).
 *
 * With `VORN_RECORD_JS_REFERENCE=1` each answer is taken from the live call
 * and written when the file is saved; otherwise it is read from the file, and
 * a key it lacks fails the test.
 */
export class JsReference {
  readonly recording = process.env.VORN_RECORD_JS_REFERENCE === '1'
  private readonly file: string
  private answers: Record<string, unknown>

  constructor(name: string) {
    this.file = path.join(__dirname, '..', 'fixtures', 'js-reference', `${name}.json`)
    this.answers =
      this.recording || !fs.existsSync(this.file)
        ? {}
        : (JSON.parse(fs.readFileSync(this.file, 'utf-8')) as Record<string, unknown>)
  }

  /** The recorded answer under `key`, or, while recording, `live`'s. */
  async want<T>(key: string, live: () => Promise<T>): Promise<T> {
    if (this.recording) {
      const answer = await live()
      this.answers[key] = answer
      return answer
    }
    if (!(key in this.answers)) throw new Error(`no recorded answer for ${key} in ${this.file}`)
    return this.answers[key] as T
  }

  /** Writes what was recorded; nothing when replaying. */
  save(): void {
    if (!this.recording) return
    fs.writeFileSync(this.file, JSON.stringify(this.answers, null, 2) + '\n')
  }
}

/** On Windows, a path's separators as the recording (made on a POSIX machine) writes them. */
export function posixSeparators<T>(value: T): T {
  if (process.platform !== 'win32') return value
  return JSON.parse(JSON.stringify(value).replaceAll('\\\\', '/')) as T
}
