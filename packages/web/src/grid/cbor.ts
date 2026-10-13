/**
 * The CBOR the grid protocol uses (packages/core/crates/term-proto/src/cbor.rs):
 * unsigned and negative integers, byte and text strings, arrays, maps keyed by
 * small integers, booleans, null and floats. The writer emits what vornd's
 * writer does, byte for byte, maps as indefinite-length maps.
 */

/** An unsigned integer: a number while it is exact, a bigint past that. */
export type Uint = number | bigint

export type CborValue =
  | Uint
  | { neg: Uint }
  | Uint8Array
  | string
  | CborValue[]
  | CborMap
  | boolean
  | null
  | { float: number }

/** A map's entries by integer key; other keys are dropped. */
export type CborMap = Map<number, CborValue>

export class CborError extends Error {}

const MAX_DEPTH = 32
const utf8 = new TextDecoder('utf-8', { fatal: true })
const encoder = new TextEncoder()

/** One CBOR item, the whole of `bytes`. */
export function decodeCbor(bytes: Uint8Array): CborValue {
  const r = new Reader(bytes)
  const v = r.item(0)
  if (r.at !== bytes.length) throw new CborError('trailing bytes')
  return v
}

class Reader {
  at = 0
  private bytes: Uint8Array
  private view: DataView

  constructor(bytes: Uint8Array) {
    this.bytes = bytes
    this.view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
  }

  private need(n: number): void {
    if (this.at + n > this.bytes.length) throw new CborError('truncated')
  }

  private take(n: number): Uint8Array {
    this.need(n)
    const s = this.bytes.subarray(this.at, this.at + n)
    this.at += n
    return s
  }

  /** The head byte and its argument; `null` for an indefinite length. */
  private head(): [number, Uint | null] {
    this.need(1)
    const b = this.bytes[this.at++]
    const info = b & 0x1f
    if (info < 24) return [b, info]
    switch (info) {
      case 24:
        return [b, this.take(1)[0]]
      case 25:
        this.take(2)
        return [b, this.view.getUint16(this.at - 2)]
      case 26:
        this.take(4)
        return [b, this.view.getUint32(this.at - 4)]
      case 27: {
        this.take(8)
        const v = this.view.getBigUint64(this.at - 8)
        return [b, v <= BigInt(Number.MAX_SAFE_INTEGER) ? Number(v) : v]
      }
      case 31:
        return [b, null]
      default:
        throw new CborError(`unsupported head 0x${b.toString(16)}`)
    }
  }

  /** A count that must fit in the bytes left, one byte an element at least. */
  private len(n: Uint): number {
    if (typeof n === 'bigint' || n > this.bytes.length - this.at) throw new CborError('too long')
    return n
  }

  item(depth: number): CborValue {
    if (depth > MAX_DEPTH) throw new CborError('too deep')
    const [b, arg] = this.head()
    const major = b >> 5
    if (arg === null) return this.indefinite(major, b, depth)
    switch (major) {
      case 0:
        return arg
      case 1:
        return { neg: arg }
      case 2:
        return this.take(this.len(arg)).slice()
      case 3:
        return text(this.take(this.len(arg)))
      case 4: {
        const n = this.len(arg)
        const items: CborValue[] = []
        for (let i = 0; i < n; i++) items.push(this.item(depth + 1))
        return items
      }
      case 5: {
        const n = this.len(arg)
        const map: CborMap = new Map()
        for (let i = 0; i < n; i++) this.entry(map, depth)
        return map
      }
      case 7:
        switch (b & 0x1f) {
          case 20:
            return false
          case 21:
            return true
          case 22:
          case 23:
            return null
          case 25:
            return { float: f16(Number(arg)) }
          case 26:
            return { float: this.view.getFloat32(this.at - 4) }
          case 27:
            return { float: this.view.getFloat64(this.at - 8) }
        }
    }
    throw new CborError(`unsupported head 0x${b.toString(16)}`)
  }

  private atBreak(): boolean {
    this.need(1)
    if (this.bytes[this.at] !== 0xff) return false
    this.at++
    return true
  }

  private indefinite(major: number, b: number, depth: number): CborValue {
    switch (major) {
      case 2:
      case 3: {
        const chunks: Uint8Array[] = []
        while (!this.atBreak()) {
          const c = this.item(depth + 1)
          if (major === 2 && c instanceof Uint8Array) chunks.push(c)
          else if (major === 3 && typeof c === 'string') chunks.push(encoder.encode(c))
          else throw new CborError('a chunk of another type')
        }
        const all = concat(chunks)
        return major === 2 ? all : text(all)
      }
      case 4: {
        const items: CborValue[] = []
        while (!this.atBreak()) items.push(this.item(depth + 1))
        return items
      }
      case 5: {
        const map: CborMap = new Map()
        while (!this.atBreak()) this.entry(map, depth)
        return map
      }
    }
    throw new CborError(`unsupported head 0x${b.toString(16)}`)
  }

  private entry(map: CborMap, depth: number): void {
    const k = this.item(depth + 1)
    const v = this.item(depth + 1)
    if (typeof k === 'number') map.set(k, v)
  }
}

function text(bytes: Uint8Array): string {
  try {
    return utf8.decode(bytes)
  } catch {
    throw new CborError('not UTF-8')
  }
}

function f16(h: number): number {
  const sign = h & 0x8000 ? -1 : 1
  const exp = (h >> 10) & 0x1f
  const frac = h & 0x3ff
  if (exp === 0) return sign * frac * 2 ** -24
  if (exp === 31) return frac === 0 ? sign * Infinity : NaN
  return sign * (1 + frac / 1024) * 2 ** (exp - 15)
}

export function concat(parts: Uint8Array[]): Uint8Array {
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0))
  let at = 0
  for (const p of parts) {
    out.set(p, at)
    at += p.length
  }
  return out
}

/** Writes CBOR the way vornd's writer does. */
export class CborWriter {
  private out: number[] = []

  private head(major: number, v: Uint): void {
    const m = major << 5
    const n = BigInt(v)
    if (n < 0n) throw new CborError('negative')
    if (n < 24n) this.out.push(m | Number(n))
    else if (n < 0x100n) this.out.push(m | 24, Number(n))
    else if (n < 0x10000n) this.out.push(m | 25, ...be(n, 2))
    else if (n < 0x100000000n) this.out.push(m | 26, ...be(n, 4))
    else this.out.push(m | 27, ...be(n, 8))
  }

  uint(v: Uint): this {
    this.head(0, v)
    return this
  }

  bytes(b: Uint8Array): this {
    this.head(2, b.length)
    for (const x of b) this.out.push(x)
    return this
  }

  text(s: string): this {
    const b = encoder.encode(s)
    this.head(3, b.length)
    for (const x of b) this.out.push(x)
    return this
  }

  bool(b: boolean): this {
    this.out.push(b ? 0xf5 : 0xf4)
    return this
  }

  f32(v: number): this {
    const b = new DataView(new ArrayBuffer(4))
    b.setFloat32(0, v)
    this.out.push(0xfa, b.getUint8(0), b.getUint8(1), b.getUint8(2), b.getUint8(3))
    return this
  }

  /** Starts a map of unknown size; `end` closes it. */
  map(): this {
    this.out.push(0xbf)
    return this
  }

  end(): this {
    this.out.push(0xff)
    return this
  }

  /** Field `key` of the open map; its value is written next. */
  key(key: number): this {
    return this.uint(key)
  }

  finish(): Uint8Array {
    return Uint8Array.from(this.out)
  }
}

function be(n: bigint, width: number): number[] {
  const out: number[] = []
  for (let i = width - 1; i >= 0; i--) out.push(Number((n >> BigInt(8 * i)) & 0xffn))
  return out
}
