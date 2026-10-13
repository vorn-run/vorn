/**
 * The grid protocol's messages (Terminal State Protocol;
 * packages/core/crates/term-proto/src/msg.rs and row.rs), as vornd's `/grid`
 * WebSocket carries them: frames of a 4-byte little-endian length, a kind
 * byte and a CBOR map keyed by field number. A frame may be split across
 * WebSocket messages, so every message goes through a {@link FrameReader}.
 */

import {
  CborError,
  CborWriter,
  concat,
  decodeCbor,
  type CborMap,
  type CborValue,
  type Uint
} from './cbor'

export const PROTO_MAJOR = 1
export const PROTO_MINOR = 0
export const CAPS = { GRID: 1, HISTORY_TABLES: 2, ALL: 3 } as const
export const ROW_FMT = 1
export const MAX_FRAME = 64 << 20

export const KIND = {
  HELLO: 0x01,
  ATTACH: 0x02,
  DETACH: 0x03,
  SET_VISIBLE: 0x04,
  ACK: 0x05,
  INPUT: 0x06,
  VIEWPORT: 0x07,
  PRESENCE: 0x08,
  TAKE_SIZE: 0x09,
  LOCK_SIZE: 0x0a,

  WELCOME: 0x21,
  ATTACHED: 0x22,
  SNAPSHOT: 0x23,
  DELTA: 0x24,
  RESET_TABLES: 0x25,
  RESIZED: 0x28,
  INPUT_ACK: 0x29,
  PASTE_CONFIRM: 0x2a,
  EVENT: 0x2f,
  RESYNC: 0x30,
  ERROR: 0x31,

  /** Kinds with this bit may be skipped by a reader that does not know them. */
  OPTIONAL: 0x80
} as const

/** The last server kind this build knows of, read or not. */
const LAST_SERVER_KIND = KIND.ERROR

export class ProtocolError extends Error {}

/** Splits the bytes of a socket into frames. */
export class FrameReader {
  private buf: Uint8Array = new Uint8Array(0)

  push(bytes: Uint8Array): void {
    this.buf = this.buf.length ? concat([this.buf, bytes]) : bytes.slice()
  }

  /** The next whole frame, if one has arrived. */
  next(): { kind: number; payload: Uint8Array } | null {
    if (this.buf.length < 4) return null
    const len = new DataView(this.buf.buffer, this.buf.byteOffset, 4).getUint32(0, true)
    if (len === 0 || len > MAX_FRAME) throw new ProtocolError(`a frame of ${len} bytes`)
    if (this.buf.length < 4 + len) return null
    const kind = this.buf[4]
    const payload = this.buf.subarray(5, 4 + len)
    this.buf = this.buf.subarray(4 + len)
    return { kind, payload }
  }
}

// ---------------------------------------------------------------------------
// Client to vornd
// ---------------------------------------------------------------------------

export const ClientKind = { Native: 0, Electron: 1, Web: 2, Cli: 3 } as const
export type ClientKind = (typeof ClientKind)[keyof typeof ClientKind]

export interface Size {
  cols: number
  rows: number
  pxW?: number
  pxH?: number
}

export interface GridResume {
  stateGen: Uint
  rev: Uint
  tableGen: number
  styleMark: number
  linkMark: number
}

export type InputEvent =
  | {
      type: 'key'
      action: 'press' | 'repeat' | 'release'
      key: number
      mods: number
      consumedMods: number
      text?: string
      unshifted?: number
      composing: boolean
    }
  | { type: 'text'; utf8: string }
  | { type: 'paste'; utf8: string; confirmed: boolean }
  | {
      type: 'mouse'
      action: 'press' | 'release' | 'motion'
      button?: number
      mods: number
      x: number
      y: number
    }
  | { type: 'wheel'; dx: number; dy: number; mods: number; x: number; y: number }
  | { type: 'focus'; focused: boolean }
  | { type: 'raw'; bytes: Uint8Array }

export type Presence = 'active' | 'watching' | 'away'

export type ClientMsg =
  | { type: 'hello'; major: number; minor: number; caps: Uint; client: ClientKind; build: string }
  | {
      type: 'attach'
      session: string
      view: Size
      visible: boolean
      resume?: GridResume
      historyTail: number
    }
  | { type: 'detach'; sid: number }
  | { type: 'setVisible'; sid: number; visible: boolean }
  | { type: 'ack'; sid: number; rev: Uint }
  | { type: 'input'; sid: number; inputSeq: Uint; event: InputEvent }
  | { type: 'viewport'; sid: number; size: Size }
  | { type: 'presence'; sid: number; state: Presence }
  | { type: 'takeSize'; sid: number }
  | { type: 'lockSize'; sid: number; locked: boolean }

const CLIENT_KIND: Record<ClientMsg['type'], number> = {
  hello: KIND.HELLO,
  attach: KIND.ATTACH,
  detach: KIND.DETACH,
  setVisible: KIND.SET_VISIBLE,
  ack: KIND.ACK,
  input: KIND.INPUT,
  viewport: KIND.VIEWPORT,
  presence: KIND.PRESENCE,
  takeSize: KIND.TAKE_SIZE,
  lockSize: KIND.LOCK_SIZE
}

function putSize(w: CborWriter, s: Size): void {
  w.map()
  w.key(1).uint(s.cols)
  w.key(2).uint(s.rows)
  w.key(3).uint(s.pxW ?? 0)
  w.key(4).uint(s.pxH ?? 0)
  w.end()
}

function putEvent(w: CborWriter, e: InputEvent): void {
  w.map()
  switch (e.type) {
    case 'key':
      w.key(1).uint(0)
      w.key(2).uint(['press', 'repeat', 'release'].indexOf(e.action))
      w.key(3).uint(e.key)
      w.key(4).uint(e.mods)
      w.key(5).uint(e.consumedMods)
      if (e.text !== undefined) w.key(6).text(e.text)
      if (e.unshifted !== undefined) w.key(7).uint(e.unshifted)
      w.key(8).bool(e.composing)
      break
    case 'text':
      w.key(1).uint(1)
      w.key(2).text(e.utf8)
      break
    case 'paste':
      w.key(1).uint(2)
      w.key(2).text(e.utf8)
      w.key(3).bool(e.confirmed)
      break
    case 'mouse':
      w.key(1).uint(3)
      w.key(2).uint(['press', 'release', 'motion'].indexOf(e.action))
      if (e.button !== undefined) w.key(3).uint(e.button)
      w.key(4).uint(e.mods)
      w.key(5).f32(e.x)
      w.key(6).f32(e.y)
      break
    case 'wheel':
      w.key(1).uint(4)
      w.key(2).f32(e.dx)
      w.key(3).f32(e.dy)
      w.key(4).uint(e.mods)
      w.key(5).f32(e.x)
      w.key(6).f32(e.y)
      break
    case 'focus':
      w.key(1).uint(5)
      w.key(2).bool(e.focused)
      break
    case 'raw':
      w.key(1).uint(6)
      w.key(2).bytes(e.bytes)
      break
  }
  w.end()
}

/** One message as a frame, the bytes vornd's own encoder writes for it. */
export function encodeClient(m: ClientMsg): Uint8Array {
  const w = new CborWriter()
  w.map()
  switch (m.type) {
    case 'hello':
      w.key(1).uint(m.major)
      w.key(2).uint(m.minor)
      w.key(3).uint(m.caps)
      w.key(4).uint(m.client)
      w.key(5).text(m.build)
      break
    case 'attach':
      w.key(1).text(m.session)
      w.key(2).uint(0)
      w.key(3)
      putSize(w, m.view)
      w.key(4).bool(m.visible)
      if (m.resume) {
        w.key(5).map()
        w.key(1).uint(m.resume.stateGen)
        w.key(2).uint(m.resume.rev)
        w.key(3).uint(m.resume.tableGen)
        w.key(4).uint(m.resume.styleMark)
        w.key(5).uint(m.resume.linkMark)
        w.end()
      }
      w.key(6).uint(m.historyTail)
      break
    case 'detach':
    case 'takeSize':
      w.key(1).uint(m.sid)
      break
    case 'setVisible':
      w.key(1).uint(m.sid)
      w.key(2).bool(m.visible)
      break
    case 'ack':
      w.key(1).uint(m.sid)
      w.key(2).uint(m.rev)
      break
    case 'input':
      w.key(1).uint(m.sid)
      w.key(2).uint(m.inputSeq)
      w.key(3)
      putEvent(w, m.event)
      break
    case 'viewport':
      w.key(1).uint(m.sid)
      w.key(2)
      putSize(w, m.size)
      break
    case 'presence':
      w.key(1).uint(m.sid)
      w.key(2).uint(['active', 'watching', 'away'].indexOf(m.state))
      break
    case 'lockSize':
      w.key(1).uint(m.sid)
      w.key(2).bool(m.locked)
      break
  }
  w.end()
  const payload = w.finish()
  const frame = new Uint8Array(5 + payload.length)
  new DataView(frame.buffer).setUint32(0, payload.length + 1, true)
  frame[4] = CLIENT_KIND[m.type]
  frame.set(payload, 5)
  return frame
}

// ---------------------------------------------------------------------------
// vornd to client
// ---------------------------------------------------------------------------

export interface Cursor {
  epoch: number
  nextRseq: Uint
  nextOffset: Uint
}

export interface CursorState {
  x: number
  y: number
  visible: boolean
  blinking: boolean
  /** 0 block, 1 hollow block, 2 bar, 3 underline. */
  style: number
  wideTail: boolean
}

export interface Colors {
  fg?: number
  bg?: number
  cursor?: number
  /** Palette entries as `[index, 0xRRGGBB]`. */
  palette: [number, number][]
}

export interface TermState {
  cols: number
  rows: number
  /** 0 primary, 1 alternate. */
  screen: number
  cursor: CursorState
  colors: Colors
  mouse: number
  flags: number
  title: string
  cwd: string
  sbEpoch: number
  historyLines: Uint
  topLine: Uint
}

export type TermDelta = Partial<Omit<TermState, 'cols' | 'rows'>> & {
  size?: { cols: number; rows: number }
}

/** A colour: absent for the default, `< 256` a palette index, else `1 << 24 | rgb`. */
export interface StyleDef {
  id: number
  fg?: number
  bg?: number
  underlineColor?: number
  attrs: number
  underline: number
}

export interface LinkDef {
  id: number
  uri: string
  osc8Id?: string
}

export interface Row {
  y: number
  line: Uint
  flags: number
  /** Cells in row format 1; see {@link decodeRow}. */
  cells: Uint8Array
}

export interface Snapshot {
  stateGen: Uint
  rev: Uint
  resume: Cursor
  tableGen: number
  rowFmt: number
  term: TermState
  styles: StyleDef[]
  links: LinkDef[]
  rows: Row[]
  history: Row[]
}

export interface Delta {
  stateGen: Uint
  tableGen: number
  baseRev: Uint
  rev: Uint
  resume: Cursor
  scrolled: number
  term?: TermDelta
  styles: StyleDef[]
  links: LinkDef[]
  rows: Row[]
}

export type EventKind =
  | { type: 'bell' }
  | { type: 'clipboard'; text: string }
  | { type: 'notify'; title: string; body: string }
  | { type: 'status'; state: number }
  | { type: 'exit'; code?: number; signal?: number }

export type ServerMsg =
  | { type: 'welcome'; major: number; minor: number; caps: Uint; build: string; instance: Uint }
  | {
      type: 'attached'
      sid: number
      session: string
      epoch: number
      owner: boolean
      exact: boolean
    }
  | { type: 'snapshot'; sid: number; snap: Snapshot }
  | { type: 'delta'; sid: number; delta: Delta }
  | { type: 'resetTables'; sid: number; tableGen: number }
  | {
      type: 'resized'
      sid: number
      cols: number
      rows: number
      rseq: Uint
      rev?: Uint
      owner?: number
      /** 0 input, 1 returned, 2 explicit, 3 locked, 4 launch. */
      reason?: number
    }
  | { type: 'inputAck'; sid: number; inputSeq: Uint }
  | { type: 'pasteConfirm'; sid: number; inputSeq: Uint }
  | {
      type: 'event'
      sid: number
      id: { epoch: number; rseq: Uint; index: number }
      kind: EventKind
    }
  /** 0 gap, 1 tables compacted, 2 base mismatch, 3 restarted, 4 not retained. */
  | { type: 'resync'; sid: number; reason: number }
  | { type: 'error'; code: number; message: string }
  /** A kind this client does not read (history, selection, copy, search, bytes). */
  | { type: 'other'; kind: number }

/** Reads one message's fields, failing as vornd's decoder does. */
class Fields {
  private kind: number
  private f: CborMap

  constructor(kind: number, v: CborValue) {
    if (!(v instanceof Map)) throw new ProtocolError(`kind 0x${kind.toString(16)}: not a map`)
    this.kind = kind
    this.f = v
  }

  private bad(key: number): ProtocolError {
    return new ProtocolError(`kind 0x${this.kind.toString(16)}: field ${key}`)
  }

  optUint(key: number): Uint | undefined {
    const v = this.f.get(key)
    if (v === undefined) return undefined
    if (typeof v !== 'number' && typeof v !== 'bigint') throw this.bad(key)
    return v
  }

  uint(key: number): Uint {
    const v = this.optUint(key)
    if (v === undefined) throw this.bad(key)
    return v
  }

  /** A number that fits in a JS number, as every count and id here does. */
  optNum(key: number): number | undefined {
    const v = this.optUint(key)
    if (typeof v === 'bigint') throw this.bad(key)
    return v
  }

  num(key: number): number {
    const v = this.optNum(key)
    if (v === undefined) throw this.bad(key)
    return v
  }

  numOrZero(key: number): number {
    return this.optNum(key) ?? 0
  }

  uintOrZero(key: number): Uint {
    return this.optUint(key) ?? 0
  }

  optInt(key: number): number | undefined {
    const v = this.f.get(key)
    if (v === undefined || v === null) return undefined
    if (typeof v === 'number') return v
    if (typeof v === 'object' && 'neg' in v && typeof v.neg === 'number') return -1 - v.neg
    throw this.bad(key)
  }

  flag(key: number): boolean {
    const v = this.f.get(key)
    if (v === undefined) return false
    if (typeof v !== 'boolean') throw this.bad(key)
    return v
  }

  optText(key: number): string | undefined {
    const v = this.f.get(key)
    if (v === undefined || v === null) return undefined
    if (typeof v !== 'string') throw this.bad(key)
    return v
  }

  text(key: number): string {
    return this.optText(key) ?? ''
  }

  bytes(key: number): Uint8Array {
    const v = this.f.get(key)
    if (v === undefined) return new Uint8Array(0)
    if (!(v instanceof Uint8Array)) throw this.bad(key)
    return v
  }

  list<T>(key: number, read: (f: Fields) => T): T[] {
    const v = this.f.get(key)
    if (v === undefined) return []
    if (!Array.isArray(v)) throw this.bad(key)
    return v.map((item) => read(new Fields(this.kind, item)))
  }

  rawList(key: number): CborValue[] {
    const v = this.f.get(key)
    if (v === undefined) return []
    if (!Array.isArray(v)) throw this.bad(key)
    return v
  }

  optMap(key: number): Fields | undefined {
    const v = this.f.get(key)
    if (v === undefined || v === null) return undefined
    return new Fields(this.kind, v)
  }

  map(key: number): Fields {
    const m = this.optMap(key)
    if (!m) throw this.bad(key)
    return m
  }
}

function getCursor(d: Fields): Cursor {
  return { epoch: d.num(1), nextRseq: d.uint(2), nextOffset: d.uint(3) }
}

function getCursorState(d: Fields): CursorState {
  return {
    x: d.numOrZero(1),
    y: d.numOrZero(2),
    visible: d.flag(3),
    blinking: d.flag(4),
    style: d.numOrZero(5),
    wideTail: d.flag(6)
  }
}

function getColors(d: Fields): Colors {
  const flat = d.rawList(4)
  if (flat.length % 2) throw new ProtocolError('an odd palette')
  const palette: [number, number][] = []
  for (let i = 0; i < flat.length; i += 2) {
    const [idx, rgb] = [flat[i], flat[i + 1]]
    if (typeof idx !== 'number' || typeof rgb !== 'number') throw new ProtocolError('a bad palette')
    palette.push([idx, rgb])
  }
  return { fg: d.optNum(1), bg: d.optNum(2), cursor: d.optNum(3), palette }
}

const NO_CURSOR: CursorState = {
  x: 0,
  y: 0,
  visible: false,
  blinking: false,
  style: 0,
  wideTail: false
}

function getTerm(d: Fields): TermState {
  const cursor = d.optMap(4)
  const colors = d.optMap(5)
  return {
    cols: d.num(1),
    rows: d.num(2),
    screen: d.numOrZero(3),
    cursor: cursor ? getCursorState(cursor) : { ...NO_CURSOR },
    colors: colors ? getColors(colors) : { palette: [] },
    mouse: d.numOrZero(6),
    flags: d.numOrZero(7),
    title: d.text(8),
    cwd: d.text(9),
    sbEpoch: d.numOrZero(10),
    historyLines: d.uintOrZero(11),
    topLine: d.uintOrZero(12)
  }
}

function getTermDelta(d: Fields): TermDelta {
  const t: TermDelta = {}
  const cols = d.optNum(1)
  const rows = d.optNum(2)
  if ((cols === undefined) !== (rows === undefined)) throw new ProtocolError('half a size')
  if (cols !== undefined && rows !== undefined) t.size = { cols, rows }
  const set = <K extends keyof TermDelta>(k: K, v: TermDelta[K] | undefined): void => {
    if (v !== undefined) t[k] = v
  }
  set('screen', d.optNum(3))
  const cursor = d.optMap(4)
  if (cursor) t.cursor = getCursorState(cursor)
  const colors = d.optMap(5)
  if (colors) t.colors = getColors(colors)
  set('mouse', d.optNum(6))
  set('flags', d.optNum(7))
  set('title', d.optText(8))
  set('cwd', d.optText(9))
  set('sbEpoch', d.optNum(10))
  set('historyLines', d.optUint(11))
  set('topLine', d.optUint(12))
  return t
}

function getStyle(d: Fields): StyleDef {
  return {
    id: d.num(1),
    fg: d.optNum(2),
    bg: d.optNum(3),
    underlineColor: d.optNum(4),
    attrs: d.numOrZero(5),
    underline: d.numOrZero(6)
  }
}

function getLink(d: Fields): LinkDef {
  return { id: d.num(1), uri: d.text(2), osc8Id: d.optText(3) }
}

function getRow(d: Fields): Row {
  return { y: d.num(1), line: d.uintOrZero(2), flags: d.numOrZero(3), cells: d.bytes(4) }
}

/**
 * One frame's message; `null` for an optional kind this build does not know,
 * which the reader skips.
 */
export function decodeServer(kind: number, payload: Uint8Array): ServerMsg | null {
  const known = kind >= KIND.WELCOME && kind <= LAST_SERVER_KIND
  if (!known) {
    if (kind & KIND.OPTIONAL) return null
    throw new ProtocolError(`unknown kind 0x${kind.toString(16)}`)
  }
  let value: CborValue
  try {
    value = decodeCbor(payload)
  } catch (e) {
    throw new ProtocolError(e instanceof CborError ? e.message : String(e))
  }
  const d = new Fields(kind, value)
  switch (kind) {
    case KIND.WELCOME:
      return {
        type: 'welcome',
        major: d.num(1),
        minor: d.numOrZero(2),
        caps: d.uintOrZero(3),
        build: d.text(4),
        instance: d.uintOrZero(5)
      }
    case KIND.ATTACHED:
      return {
        type: 'attached',
        sid: d.num(1),
        session: d.text(2),
        epoch: d.numOrZero(3),
        owner: d.flag(4),
        exact: (d.optNum(5) ?? 1) === 0
      }
    case KIND.SNAPSHOT:
      return {
        type: 'snapshot',
        sid: d.num(1),
        snap: {
          stateGen: d.uint(2),
          rev: d.uint(3),
          resume: getCursor(d.map(4)),
          tableGen: d.numOrZero(5),
          rowFmt: d.num(6),
          term: getTerm(d.map(7)),
          styles: d.list(8, getStyle),
          links: d.list(9, getLink),
          rows: d.list(10, getRow),
          history: d.list(11, getRow)
        }
      }
    case KIND.DELTA: {
      const term = d.optMap(8)
      return {
        type: 'delta',
        sid: d.num(1),
        delta: {
          stateGen: d.uint(2),
          tableGen: d.numOrZero(3),
          baseRev: d.uint(4),
          rev: d.uint(5),
          resume: getCursor(d.map(6)),
          scrolled: d.numOrZero(7),
          term: term ? getTermDelta(term) : undefined,
          styles: d.list(9, getStyle),
          links: d.list(10, getLink),
          rows: d.list(11, getRow)
        }
      }
    }
    case KIND.RESET_TABLES:
      return { type: 'resetTables', sid: d.num(1), tableGen: d.numOrZero(2) }
    case KIND.RESIZED:
      return {
        type: 'resized',
        sid: d.num(1),
        cols: d.numOrZero(2),
        rows: d.numOrZero(3),
        rseq: d.uintOrZero(4),
        rev: d.optUint(5),
        owner: d.optNum(6),
        reason: d.optNum(7)
      }
    case KIND.INPUT_ACK:
      return { type: 'inputAck', sid: d.num(1), inputSeq: d.uint(2) }
    case KIND.PASTE_CONFIRM:
      return { type: 'pasteConfirm', sid: d.num(1), inputSeq: d.uint(2) }
    case KIND.EVENT: {
      const id = d.map(2)
      const e = d.map(3)
      let k: EventKind
      switch (e.num(1)) {
        case 0:
          k = { type: 'bell' }
          break
        case 1:
          k = { type: 'clipboard', text: e.text(2) }
          break
        case 2:
          k = { type: 'notify', title: e.text(2), body: e.text(3) }
          break
        case 3:
          k = { type: 'status', state: e.numOrZero(2) }
          break
        case 4:
          k = { type: 'exit', code: e.optInt(2), signal: e.optInt(3) }
          break
        default:
          throw new ProtocolError('an unknown event')
      }
      return {
        type: 'event',
        sid: d.num(1),
        id: { epoch: id.numOrZero(1), rseq: id.uintOrZero(2), index: id.numOrZero(3) },
        kind: k
      }
    }
    case KIND.RESYNC:
      return { type: 'resync', sid: d.num(1), reason: d.numOrZero(2) }
    case KIND.ERROR:
      return { type: 'error', code: d.numOrZero(1), message: d.text(2) }
    default:
      return { type: 'other', kind }
  }
}

// ---------------------------------------------------------------------------
// Row cells, row format 1
// ---------------------------------------------------------------------------

/** One cell: its grapheme cluster, empty for a cell nothing was written to. */
export interface Cell {
  text: string
  /** Takes two columns; the spacer after it is implied. */
  wide: boolean
}

export interface Run {
  style: number
  link: number
  cells: Cell[]
}

const LEN_ESCAPE = 63
const WIDE = 0x40
const RESERVED = 0x80
const MAX_CLUSTER_BYTES = 4096
const cellText = new TextDecoder('utf-8', { fatal: true })

function readVarint(b: Uint8Array, at: { i: number }): number {
  let v = 0
  for (let shift = 0; shift < 53; shift += 7) {
    if (at.i >= b.length) throw new ProtocolError('a truncated row')
    const byte = b[at.i++]
    v += (byte & 0x7f) * 2 ** shift
    if (!(byte & 0x80)) return v
  }
  throw new ProtocolError('an overlong varint')
}

/**
 * A row's runs. Trailing blank cells in the default style are not sent; the
 * row's width is the terminal's.
 */
export function decodeRow(cells: Uint8Array): Run[] {
  const runs: Run[] = []
  const at = { i: 0 }
  while (at.i < cells.length) {
    const style = readVarint(cells, at)
    const link = readVarint(cells, at)
    const head = readVarint(cells, at)
    const n = Math.floor(head / 2)
    if (n > cells.length - at.i) throw new ProtocolError('a run longer than its row')
    const run: Run = { style, link, cells: [] }
    if (head % 2 === 0) {
      for (const b of cells.subarray(at.i, at.i + n)) {
        if (b !== 0 && (b < 0x20 || b > 0x7e)) throw new ProtocolError('not ascii')
        run.cells.push({ text: b === 0 ? '' : String.fromCharCode(b), wide: false })
      }
      at.i += n
    } else {
      for (let c = 0; c < n; c++) {
        if (at.i >= cells.length) throw new ProtocolError('a truncated row')
        const h = cells[at.i++]
        if (h & RESERVED) throw new ProtocolError('a reserved cell bit')
        const len = (h & LEN_ESCAPE) === LEN_ESCAPE ? readVarint(cells, at) : h & LEN_ESCAPE
        if (len > MAX_CLUSTER_BYTES || len > cells.length - at.i) {
          throw new ProtocolError('a cell longer than its row')
        }
        let text: string
        try {
          text = cellText.decode(cells.subarray(at.i, at.i + len))
        } catch {
          throw new ProtocolError('a cell that is not UTF-8')
        }
        at.i += len
        run.cells.push({ text, wide: (h & WIDE) !== 0 })
      }
    }
    runs.push(run)
  }
  return runs
}

/** A row's text: empty cells as spaces, trailing spaces trimmed. */
export function rowText(row: Row): string {
  let s = ''
  for (const run of decodeRow(row.cells)) {
    for (const cell of run.cells) s += cell.text || ' '
  }
  return s.replace(/ +$/, '')
}
