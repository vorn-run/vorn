/**
 * Just enough H.264 Annex B to hand a companion's stream to WebCodecs.
 *
 * Nothing here decodes a picture. The stream arrives as NAL units behind
 * `00 00 01` or `00 00 00 01` start codes; a `VideoDecoder` with no
 * `description` takes them in that form, but wants one chunk per access unit
 * (one picture) and needs to know which chunks are key frames and what codec
 * string to configure. That is all this works out.
 */

export const NAL_SLICE = 1
export const NAL_IDR = 5
export const NAL_SEI = 6
export const NAL_SPS = 7
export const NAL_PPS = 8
export const NAL_AUD = 9

export function nalType(nal: Uint8Array): number {
  return nal[0] & 0x1f
}

function isVcl(type: number): boolean {
  return type === NAL_SLICE || type === NAL_IDR
}

/** Where the start code at `i` ends, or -1 when there is none at `i`. */
function startCodeEnd(bytes: Uint8Array, i: number): number {
  if (bytes[i] !== 0 || bytes[i + 1] !== 0) return -1
  if (bytes[i + 2] === 1) return i + 3
  if (bytes[i + 2] === 0 && bytes[i + 3] === 1) return i + 4
  return -1
}

/**
 * The NAL units in `bytes`, without their start codes, or null when `bytes`
 * does not begin with a start code (it is the middle of something).
 */
export function splitNals(bytes: Uint8Array): Uint8Array[] | null {
  let start = startCodeEnd(bytes, 0)
  if (start < 0) return null
  const out: Uint8Array[] = []
  let i = start
  while (i + 2 < bytes.length) {
    const end = bytes[i] === 0 && bytes[i + 1] === 0 ? startCodeEnd(bytes, i) : -1
    if (end < 0) {
      i++
      continue
    }
    if (i > start) out.push(bytes.subarray(start, i))
    start = end
    i = end
  }
  if (bytes.length > start) out.push(bytes.subarray(start))
  return out
}

/** One picture's NAL units, and whether it can be decoded on its own. */
export interface AccessUnit {
  nals: Uint8Array[]
  key: boolean
}

/**
 * Groups NAL units into access units.
 *
 * A new one starts at an access unit delimiter, SEI, SPS or PPS that follows a
 * slice, and at a slice whose `first_mb_in_slice` is 0 (the first bit of its
 * header, a ue(v) of 0, is a 1) when the current unit already has a slice. A
 * unit with no slice, such as parameter sets on their own, is carried into the
 * next one.
 */
export function accessUnits(nals: Uint8Array[]): { units: AccessUnit[]; rest: Uint8Array[] } {
  const units: AccessUnit[] = []
  let current: Uint8Array[] = []
  let hasSlice = false
  let key = false
  const flush = (): void => {
    units.push({ nals: current, key })
    current = []
    hasSlice = false
    key = false
  }
  for (const nal of nals) {
    if (nal.length === 0) continue
    const type = nalType(nal)
    if (hasSlice) {
      const startsPicture =
        type === NAL_AUD ||
        type === NAL_SEI ||
        type === NAL_SPS ||
        type === NAL_PPS ||
        (isVcl(type) && nal.length > 1 && (nal[1] & 0x80) !== 0)
      if (startsPicture) flush()
    }
    current.push(nal)
    if (isVcl(type)) {
      hasSlice = true
      if (type === NAL_IDR) key = true
    }
  }
  if (hasSlice) flush()
  return { units, rest: current }
}

/** `avc1.PPCCLL` from a sequence parameter set: profile, constraint flags, level. */
export function codecString(sps: Uint8Array): string | null {
  if (sps.length < 4 || nalType(sps) !== NAL_SPS) return null
  const hex = (b: number): string => b.toString(16).padStart(2, '0')
  return `avc1.${hex(sps[1])}${hex(sps[2])}${hex(sps[3])}`
}

/** The units back in Annex B form, each behind a four-byte start code. */
export function annexB(nals: Uint8Array[]): Uint8Array {
  let size = 0
  for (const n of nals) size += 4 + n.length
  const out = new Uint8Array(size)
  let at = 0
  for (const n of nals) {
    out.set([0, 0, 0, 1], at)
    out.set(n, at + 4)
    at += 4 + n.length
  }
  return out
}
