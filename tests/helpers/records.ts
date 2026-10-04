import type { RecordCursor } from '../../packages/shared/src/types'
import { startHistory, recordOutput, recordResize } from '../../packages/server/src/history/writer'
import {
  frameData,
  frameResize,
  type LogRecord,
  type RecordHeader
} from '../../packages/server/src/history/log'

/**
 * The PTY reader's numbering, for tests that drive the history writer without
 * a PtyManager: each session's records numbered from where its log started,
 * with offsets in UTF-8 bytes, exactly as `PtyManager.nextRecord` does.
 */
const cursors = new Map<string, RecordCursor>()

/** The epoch every session starts in here; any fixed number does. */
export const EPOCH = 1

/** `startHistory`, with the cursor a fresh process starts at. */
export function startRecording(id: string): RecordCursor {
  const start = { epoch: EPOCH, nextRseq: 0, nextOffset: 0 }
  cursors.set(id, { ...start })
  startHistory(id, start)
  return start
}

function next(id: string, bytes: number): RecordHeader {
  let at = cursors.get(id)
  if (!at) {
    at = { epoch: EPOCH, nextRseq: 0, nextOffset: 0 }
    cursors.set(id, at)
  }
  const header = { rseq: at.nextRseq, startOffset: at.nextOffset }
  at.nextRseq += 1
  at.nextOffset += bytes
  return header
}

export function recordText(id: string, data: string): void {
  recordOutput(id, next(id, Buffer.byteLength(data, 'utf-8')), data)
}

export function recordSize(id: string, cols: number, rows: number): void {
  recordResize(id, next(id, 0), cols, rows)
}

/** Where this session's numbering has reached. */
export function cursorOf(id: string): RecordCursor | undefined {
  const at = cursors.get(id)
  return at ? { ...at } : undefined
}

export function resetRecording(): void {
  cursors.clear()
}

/** The output in a list of records, joined, for assertions about content. */
export function textOf(records: LogRecord[]): string {
  return records
    .filter((r): r is Extract<LogRecord, { kind: 'data' }> => r.kind === 'data')
    .map((r) => r.data)
    .join('')
}

/**
 * Framed records numbered on from `start`, for building a log by hand. A string
 * is output; a pair is a resize.
 */
export function framesFrom(
  start: RecordCursor,
  ...items: Array<string | [number, number]>
): Buffer[] {
  let rseq = start.nextRseq
  let offset = start.nextOffset
  return items.map((item) => {
    const at = { rseq: rseq++, startOffset: offset }
    if (typeof item !== 'string') return frameResize(at, item[0], item[1])
    const bytes = Buffer.from(item, 'utf-8')
    offset += bytes.length
    return frameData(at, bytes)
  })
}
