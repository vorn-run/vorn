/**
 * What vornd answers in a run against a real server, kept as a fixture under
 * `tests/fixtures/vornd/` so a change to it shows. `VORN_RECORD_FIXTURES=1`
 * writes them again from a run, for `prettier --write` to format.
 */
import fs from 'node:fs'
import path from 'node:path'

/** The fixture `name`, after writing `seen` to it when recording. */
export function recorded(name: string, seen: unknown): unknown {
  const file = path.join(__dirname, '..', 'fixtures', 'vornd', `${name}.json`)
  if (process.env.VORN_RECORD_FIXTURES === '1') {
    fs.writeFileSync(file, `${JSON.stringify(seen, null, 2)}\n`)
  }
  return JSON.parse(fs.readFileSync(file, 'utf-8'))
}
