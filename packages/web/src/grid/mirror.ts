/**
 * One session's screen built from snapshots and deltas, the way vornd's own
 * mirror (packages/core/crates/term-mirror) applies them: enough to read the
 * viewport's text and state. Scrollback and table bookkeeping are left to the
 * renderer that adopts this.
 */

import type { Uint } from './cbor'
import {
  rowText,
  type Delta,
  type LinkDef,
  type Row,
  type Snapshot,
  type StyleDef,
  type TermState
} from './protocol'

export type Refusal = 'stateGen' | 'tableGen' | 'base' | 'rowOutOfRange'

export class GridMirror {
  term: TermState
  rows: Row[]
  rev: Uint
  styles = new Map<number, StyleDef>()
  links = new Map<number, LinkDef>()
  private stateGen: Uint
  private tableGen: number

  constructor(snap: Snapshot) {
    this.term = structuredClone(snap.term)
    this.rev = snap.rev
    this.stateGen = snap.stateGen
    this.tableGen = snap.tableGen
    this.rows = blankRows(0, snap.term.rows)
    this.addTables(snap.styles, snap.links)
    for (const r of snap.rows) this.place(r)
  }

  /** Applies a delta, or refuses it whole and leaves the screen as it was. */
  apply(d: Delta): Refusal | null {
    if (d.stateGen !== this.stateGen) return 'stateGen'
    if (d.tableGen !== this.tableGen) return 'tableGen'
    if (d.baseRev !== this.rev) return 'base'
    const height = d.term?.size?.rows ?? this.term.rows
    if (d.rows.some((r) => r.y >= height)) return 'rowOutOfRange'

    this.addTables(d.styles, d.links)
    if (d.term) {
      const { size, ...rest } = d.term
      Object.assign(this.term, structuredClone(rest))
      if (size) {
        this.term.cols = size.cols
        this.term.rows = size.rows
        this.rows = this.rows.slice(0, size.rows)
        this.rows.push(...blankRows(this.rows.length, size.rows - this.rows.length))
        this.rows = this.rows.map((r, y) => ({ ...r, y }))
      }
    }
    if (d.scrolled > 0 && this.term.screen === 0) {
      const n = Math.min(d.scrolled, this.rows.length)
      this.rows = this.rows.slice(n)
      this.rows.push(...blankRows(0, n))
      this.rows = this.rows.map((r, y) => ({ ...r, y }))
    }
    for (const r of d.rows) this.place(r)
    this.rev = d.rev
    return null
  }

  /** The viewport's text, one line a row, trailing spaces trimmed. */
  text(): string {
    return this.rows.map(rowText).join('\n')
  }

  private addTables(styles: StyleDef[], links: LinkDef[]): void {
    for (const s of styles) this.styles.set(s.id, s)
    for (const l of links) this.links.set(l.id, l)
  }

  private place(r: Row): void {
    if (r.y < this.rows.length) this.rows[r.y] = r
  }
}

function blankRows(from: number, n: number): Row[] {
  return Array.from({ length: Math.max(0, n) }, (_, i) => ({
    y: from + i,
    line: 0,
    flags: 0,
    cells: new Uint8Array(0)
  }))
}
