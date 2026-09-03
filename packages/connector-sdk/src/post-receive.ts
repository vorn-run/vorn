import type { PostReceiveOp } from './types'

/**
 * Reshaping a response, declaratively.
 *
 * Most connector actions differ from each other only in what they ask for and
 * which part of the answer is worth keeping — an envelope to unwrap, a few
 * fields to keep, a name to correct. Written by hand that is a page of
 * defensive property access per action; written as these five ops it is a
 * line each, and every one of them is a pure function this file can test
 * without a network.
 */

/** Keys that would reach through an object into its prototype. */
const UNSAFE_KEYS = new Set(['__proto__', 'constructor', 'prototype'])

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function segments(path: string): string[] {
  return path
    .split('.')
    .map((part) => part.trim())
    .filter((part) => part !== '')
}

/** The value at a dotted path, or undefined if any step is missing. */
export function valueAt(value: unknown, path: string): unknown {
  let current: unknown = value
  for (const key of segments(path)) {
    if (UNSAFE_KEYS.has(key)) return undefined
    if (Array.isArray(current)) {
      const index = Number(key)
      if (!Number.isInteger(index)) return undefined
      current = current[index]
      continue
    }
    if (!isRecord(current)) return undefined
    current = Object.prototype.hasOwnProperty.call(current, key) ? current[key] : undefined
  }
  return current
}

/** A copy of `value` with the dotted path replaced. Missing steps are created. */
function withValueAt(value: unknown, path: string, next: unknown): unknown {
  const keys = segments(path)
  if (keys.length === 0) return next
  const [head, ...rest] = keys
  if (UNSAFE_KEYS.has(head)) return value

  if (Array.isArray(value)) {
    const index = Number(head)
    if (!Number.isInteger(index)) return value
    const copy = [...value]
    copy[index] = rest.length === 0 ? next : withValueAt(copy[index], rest.join('.'), next)
    return copy
  }

  const base = isRecord(value) ? value : {}
  return {
    ...base,
    [head]: rest.length === 0 ? next : withValueAt(base[head], rest.join('.'), next)
  }
}

function pick(value: unknown, keys: string[]): unknown {
  if (Array.isArray(value)) return value.map((entry) => pick(entry, keys))
  if (!isRecord(value)) return value
  const out: Record<string, unknown> = {}
  for (const key of keys) {
    if (UNSAFE_KEYS.has(key)) continue
    if (Object.prototype.hasOwnProperty.call(value, key)) out[key] = value[key]
  }
  return out
}

function rename(value: unknown, from: string, to: string): unknown {
  if (Array.isArray(value)) return value.map((entry) => rename(entry, from, to))
  if (!isRecord(value)) return value
  if (UNSAFE_KEYS.has(from) || UNSAFE_KEYS.has(to)) return value
  if (!Object.prototype.hasOwnProperty.call(value, from)) return value
  const out: Record<string, unknown> = {}
  // Rebuilt in order so the renamed field keeps its place rather than moving
  // to the end, which matters when the result is read by a person.
  for (const [key, entry] of Object.entries(value)) {
    if (key === from) out[to] = entry
    else if (key !== to) out[key] = entry
  }
  return out
}

/** Apply one op to the whole value, or to what lives at its `path`. */
function applyOp(value: unknown, op: PostReceiveOp): unknown {
  if (op.op === 'flatten') return valueAt(value, op.path)

  const target = op.path === undefined ? value : valueAt(value, op.path)
  // A path that names nothing leaves the value as it was, rather than adding
  // the key it was looking for.
  if (op.path !== undefined && target === undefined) return value
  let next: unknown

  if (op.op === 'pick') next = pick(target, op.keys)
  else if (op.op === 'rename') next = rename(target, op.from, op.to)
  else if (op.op === 'filter') {
    // A filter on something that is not a list is a no-op rather than an
    // error: an upstream that answered with one object instead of a page of
    // them should not fail the step.
    next = Array.isArray(target)
      ? target.filter((entry) => isRecord(entry) && valueAt(entry, op.key) === op.equals)
      : target
  } else {
    next = Array.isArray(target) ? target.map((entry) => applyPostReceive(entry, op.ops)) : target
  }

  return op.path === undefined ? next : withValueAt(value, op.path, next)
}

/** Run a response through the declared ops, left to right. */
export function applyPostReceive(value: unknown, ops: PostReceiveOp[] | undefined): unknown {
  return (ops ?? []).reduce<unknown>(applyOp, value)
}
