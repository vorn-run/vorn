import { useEffect, useMemo, useRef, useState, type KeyboardEvent } from 'react'
import { Check, Pencil, X } from 'lucide-react'
import { isRecordList, jsonErrorLocation, toItemList } from '@vornrun/shared/item-list'
import { GATE_APPROVE, GATE_NEUTRAL } from '../../lib/gate-affordance'
import { CodeEditor } from '../code-editor/CodeEditor'

/** What the reviewer has in front of them: what it would send, and why it cannot be sent yet. */
export type GateDraft = {
  /** The rewrite to send, or undefined when it says what the gate already had. */
  edited: string | undefined
  /** Why the draft cannot be saved or approved, or null when it can. */
  invalid: string | null
}

type Row = {
  values: Record<string, unknown>
  removed: boolean
  /**
   * What was typed into number cells, by key. A half-typed number such as
   * "1." or "-" has to stay on screen as typed, and one that never parses has
   * to be told apart from the number the cell last held.
   */
  typed: Record<string, string>
  /** The keys the row came with, and which of them held null, for what emptying a cell means. */
  had: Set<string>
  nullable: Set<string>
}

type Parsed = {
  rows: Row[]
  /** The object the list sat in, and the key it sat under, to put it back where it was. */
  wrapper?: { object: Record<string, unknown>; key: string }
}

type ParseError = { error: string; line?: number }

/**
 * The rows a table can draw from `text`, or why it cannot. An empty list
 * counts: removing every row and looking at the source must not lock the
 * reviewer out of the table they came from.
 */
function parseRows(text: string): Parsed | ParseError {
  let data: unknown
  try {
    data = JSON.parse(text)
  } catch (err) {
    const where = jsonErrorLocation(text, err)
    return {
      error: `Line ${where.line}, column ${where.column}: ${where.message}`,
      line: where.line
    }
  }
  const list = toItemList(data)
  if ('error' in list) return { error: list.error }
  if (list.items.length > 0 && !isRecordList(list.items)) {
    return { error: 'Not a table: every item has to be an object.' }
  }
  const rows = (list.items as Record<string, unknown>[]).map((values) => ({
    values: { ...values },
    removed: false,
    typed: {},
    had: new Set(Object.keys(values)),
    nullable: new Set(Object.keys(values).filter((k) => values[k] === null))
  }))
  return list.wrapperKey
    ? { rows, wrapper: { object: data as Record<string, unknown>, key: list.wrapperKey } }
    : { rows }
}

/** Every key any row has, in the order they were first seen. */
function columnsOf(rows: Row[]): string[] {
  const seen = new Set<string>()
  for (const row of rows) for (const key of Object.keys(row.values)) seen.add(key)
  return [...seen]
}

function parseNumber(text: string): number | undefined {
  if (text.trim() === '') return undefined
  const n = Number(text)
  return Number.isFinite(n) ? n : undefined
}

/** The value the kept rows make, wrapped back in the object they came from. */
function valueOf(parsed: Parsed): unknown {
  const kept = parsed.rows.filter((r) => !r.removed).map((r) => r.values)
  return parsed.wrapper ? { ...parsed.wrapper.object, [parsed.wrapper.key]: kept } : kept
}

/** The first kept cell that holds something other than a number it should. */
function badCell(rows: Row[]): string | null {
  for (let i = 0; i < rows.length; i++) {
    if (rows[i].removed) continue
    for (const [key, text] of Object.entries(rows[i].typed)) {
      if (parseNumber(text) === undefined) return `Row ${i + 1}, ${key}: not a number.`
    }
  }
  return null
}

/** Same value, whatever the spacing; unparseable text only matches itself. */
function canonical(text: string): string {
  try {
    return JSON.stringify(JSON.parse(text))
  } catch {
    return text
  }
}

function Cell({
  row,
  index,
  column,
  onChange
}: {
  row: Row
  index: number
  column: string
  onChange: (next: Row) => void
}) {
  const has = Object.prototype.hasOwnProperty.call(row.values, column)
  const value = row.values[column]
  const label = `${column}, row ${index + 1}`
  // A form control does not inherit its row's strike-through, so a removed row strikes each one.
  const input = `w-full min-w-0 px-1.5 py-1 rounded-[4px] bg-transparent border border-transparent text-[12px] text-ink placeholder:text-ink-faint hover:border-white/[0.08] focus:border-white/[0.2] focus:outline-none disabled:pointer-events-none ${row.removed ? 'line-through' : ''}`
  const set = (next: Partial<Row>): void => onChange({ ...row, ...next })

  if (has && typeof value === 'boolean') {
    return (
      <label className="flex items-center gap-1.5 px-1.5 py-1 text-[12px] font-mono text-ink-secondary">
        <input
          type="checkbox"
          aria-label={label}
          checked={value}
          disabled={row.removed}
          onChange={(e) => set({ values: { ...row.values, [column]: e.target.checked } })}
          className="h-3 w-3 accent-[var(--color-ink-secondary)]"
        />
        {String(value)}
      </label>
    )
  }

  if (has && typeof value === 'number') {
    const typed = row.typed[column]
    const bad = typed !== undefined && parseNumber(typed) === undefined
    return (
      <input
        type="text"
        inputMode="decimal"
        aria-label={label}
        aria-invalid={bad || undefined}
        title={bad ? 'Not a number' : undefined}
        value={typed ?? String(value)}
        disabled={row.removed}
        onChange={(e) => {
          const text = e.target.value
          const n = parseNumber(text)
          set({
            typed: { ...row.typed, [column]: text },
            // The cell keeps the last number it held, so an invalid one never reaches the JSON.
            values: n === undefined ? row.values : { ...row.values, [column]: n }
          })
        }}
        className={`${input} font-mono tabular-nums ${bad ? '!border-danger/60 text-danger' : ''}`}
      />
    )
  }

  if (has && value !== null && typeof value === 'object') {
    const json = JSON.stringify(value)
    return (
      <span
        title="Edit in Source"
        className="block truncate px-1.5 py-1 text-[12px] font-mono text-ink-faint"
      >
        {json}
      </span>
    )
  }

  // Strings, nulls, and keys this row does not have yet.
  return (
    <input
      type="text"
      aria-label={label}
      value={has && value !== null ? String(value) : ''}
      placeholder={has && value === null ? 'null' : undefined}
      disabled={row.removed}
      onChange={(e) => {
        const text = e.target.value
        const values = { ...row.values }
        if (text === '' && row.nullable.has(column)) values[column] = null
        // A key the row never had goes again when it is emptied, so looking is not editing.
        else if (text === '' && !row.had.has(column)) delete values[column]
        else values[column] = text
        set({ values })
      }}
      className={input}
    />
  )
}

/**
 * A list of records the gate hands on, as a table the reviewer can trim and
 * correct, with the JSON behind it one toggle away.
 *
 * The table is the common case — drop the findings that are wrong, fix a
 * field — and it cannot produce broken JSON. The source is there for what a
 * table cannot say, a nested value or a new row, and while it does not parse
 * the table stays shut, because there is nothing to draw it from.
 */
export function GateJsonEditor({
  original,
  initial,
  onSave,
  onCancel,
  onApprove,
  onDraftChange,
  large
}: {
  /** The text the gate had, which an edit is measured against. */
  original: string
  /** Where the editor opens: a rewrite not yet sent, or the original. */
  initial: string
  onSave: (edited: string | undefined) => void
  onCancel: () => void
  /** Approve straight from the editor, with what it holds. */
  onApprove?: (edited: string | undefined) => void
  onDraftChange?: (draft: GateDraft) => void
  large?: boolean
}) {
  const [parsed, setParsed] = useState<Parsed | null>(() => {
    const p = parseRows(initial)
    return 'error' in p ? null : p
  })
  const [mode, setMode] = useState<'table' | 'source'>(() => (parsed ? 'table' : 'source'))
  const [source, setSource] = useState(initial)

  const columns = useMemo(() => (parsed ? columnsOf(parsed.rows) : []), [parsed])
  const sourceParse = useMemo(() => (mode === 'source' ? parseRows(source) : null), [mode, source])
  const sourceError = sourceParse && 'error' in sourceParse ? sourceParse.error : null

  const invalid = mode === 'source' ? sourceError : parsed ? badCell(parsed.rows) : null
  const text = useMemo(() => {
    if (mode === 'source') {
      return sourceParse && !('error' in sourceParse)
        ? JSON.stringify(JSON.parse(source), null, 2)
        : null
    }
    return parsed ? JSON.stringify(valueOf(parsed), null, 2) : null
  }, [mode, source, sourceParse, parsed])
  const edited = text !== null && canonical(text) !== canonical(original) ? text : undefined

  // The host hands a fresh callback each render; telling it only when the draft
  // changes keeps its state update from rendering the editor again, forever.
  const draftRef = useRef(onDraftChange)
  useEffect(() => {
    draftRef.current = onDraftChange
  })
  useEffect(() => {
    draftRef.current?.({ edited, invalid })
  }, [edited, invalid])

  const toSource = (): void => {
    if (mode === 'source') return
    if (parsed) setSource(JSON.stringify(valueOf(parsed), null, 2))
    setMode('source')
  }
  const toTable = (): void => {
    if (mode === 'table' || !sourceParse || 'error' in sourceParse) return
    setParsed(sourceParse)
    setMode('table')
  }
  const revert = (): void => {
    const p = parseRows(original)
    setParsed('error' in p ? null : p)
    setSource(original)
    setMode('error' in p ? 'source' : 'table')
  }
  const save = (): void => {
    if (!invalid) onSave(edited)
  }

  const rows = parsed?.rows ?? []
  const kept = rows.filter((r) => !r.removed).length
  const count =
    mode === 'source'
      ? sourceParse && !('error' in sourceParse)
        ? `${sourceParse.rows.length} of ${sourceParse.rows.length} kept`
        : '—'
      : `${kept} of ${rows.length} kept`
  const size = large ? 'px-3.5 py-2 text-[12.5px]' : 'px-2 py-1 text-[11px]'
  const icon = large ? 13 : 11
  const height = large ? 420 : 280
  const grid = { gridTemplateColumns: `20px repeat(${columns.length}, minmax(112px, 1fr)) 44px` }
  const tab = (active: boolean): string =>
    `px-2 py-0.5 rounded-[4px] text-[11px] transition-colors disabled:opacity-40 disabled:cursor-not-allowed ${
      active ? 'bg-white/[0.08] text-ink' : 'text-ink-faint hover:text-ink-secondary'
    }`

  const updateRow = (index: number, next: Row): void =>
    setParsed((p) => (p ? { ...p, rows: p.rows.map((r, i) => (i === index ? next : r)) } : p))

  return (
    <div
      className="flex flex-col gap-2"
      onKeyDown={(e: KeyboardEvent) => {
        if (e.key === 'Escape') {
          e.stopPropagation()
          onCancel()
        } else if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
          e.preventDefault()
          save()
        }
      }}
    >
      <div className="rounded-md border border-white/[0.08] bg-surface-panel overflow-hidden">
        <div className="flex items-center gap-2 px-3 py-1.5 border-b border-white/[0.06]">
          <span className="text-[12px] text-ink">Items</span>
          <span className="text-[11px] font-mono tabular-nums text-ink-faint">{count}</span>
          <span className="flex-1" />
          <div
            role="group"
            aria-label="View"
            className="flex items-center gap-0.5 p-0.5 rounded-md border border-white/[0.08]"
          >
            <button
              type="button"
              aria-pressed={mode === 'table'}
              disabled={mode === 'source' && (!sourceParse || 'error' in sourceParse)}
              title={
                mode === 'source' && sourceError ? 'Fix the JSON to see it as a table' : undefined
              }
              onClick={toTable}
              className={tab(mode === 'table')}
            >
              Table
            </button>
            <button
              type="button"
              aria-pressed={mode === 'source'}
              onClick={toSource}
              className={tab(mode === 'source')}
            >
              Source
            </button>
          </div>
        </div>

        {mode === 'table' ? (
          <div className="overflow-auto" style={{ maxHeight: height }}>
            {rows.length === 0 ? (
              <div className="px-3 py-4 text-[12px] text-ink-faint">No items.</div>
            ) : (
              <div className="min-w-max" role="table" aria-label="Items">
                <div
                  role="row"
                  className="grid gap-x-2 items-center px-3 py-1.5 border-b border-white/[0.06] sticky top-0 z-10 bg-surface-panel"
                  style={grid}
                >
                  <span />
                  {columns.map((c) => (
                    <span
                      key={c}
                      role="columnheader"
                      className="truncate px-1.5 text-[11px] font-mono text-ink-faint"
                      title={c}
                    >
                      {c}
                    </span>
                  ))}
                  <span />
                </div>
                {rows.map((row, i) => (
                  <div
                    key={i}
                    role="row"
                    data-removed={row.removed || undefined}
                    className={`grid gap-x-2 items-center px-3 py-2 border-b border-white/[0.04] ${
                      row.removed ? '' : 'hover:bg-white/[0.02]'
                    }`}
                    style={grid}
                  >
                    <span
                      className={`text-[11px] font-mono tabular-nums text-ink-faint ${row.removed ? 'opacity-40 line-through' : ''}`}
                    >
                      {i + 1}
                    </span>
                    {columns.map((c) => (
                      <div
                        key={c}
                        role="cell"
                        className={`min-w-0 ${row.removed ? 'opacity-40 line-through' : ''}`}
                      >
                        <Cell
                          row={row}
                          index={i}
                          column={c}
                          onChange={(next) => updateRow(i, next)}
                        />
                      </div>
                    ))}
                    <span className="flex justify-end">
                      {row.removed ? (
                        <button
                          type="button"
                          aria-label={`Undo removing row ${i + 1}`}
                          onClick={() => updateRow(i, { ...row, removed: false })}
                          className="px-1.5 py-0.5 rounded-[4px] text-[11px] text-ink-secondary hover:text-ink hover:bg-white/[0.04] transition-colors"
                        >
                          Undo
                        </button>
                      ) : (
                        <button
                          type="button"
                          aria-label={`Remove row ${i + 1}`}
                          title="Remove"
                          onClick={() => updateRow(i, { ...row, removed: true })}
                          className="p-1 rounded-[4px] text-ink-faint hover:text-danger hover:bg-danger/10 transition-colors"
                        >
                          <X size={12} strokeWidth={2} />
                        </button>
                      )}
                    </span>
                  </div>
                ))}
              </div>
            )}
          </div>
        ) : (
          <>
            <CodeEditor
              value={source}
              onChange={setSource}
              language="json"
              ariaLabel="The text to approve"
              errorLine={sourceParse && 'error' in sourceParse ? sourceParse.line : undefined}
              className=""
              minHeight={120}
              maxHeight={height}
            />
            <div
              role="status"
              className={`px-3 py-1.5 border-t border-white/[0.06] text-[11px] font-mono ${
                sourceError ? 'text-danger' : 'text-status-sage'
              }`}
            >
              {sourceError ??
                `Valid JSON · ${sourceParse && !('error' in sourceParse) ? sourceParse.rows.length : 0} items`}
            </div>
          </>
        )}
      </div>

      <div className="flex items-center gap-2 text-[11px] leading-[1.45] text-ink-faint">
        {edited !== undefined && <span className="text-ink-secondary">Edited</span>}
        {mode === 'table' && invalid && <span className="text-danger">{invalid}</span>}
        <span className="flex-1" />
        <button
          type="button"
          onClick={revert}
          disabled={edited === undefined && invalid === null}
          className="hover:text-ink-secondary transition-colors disabled:opacity-40 disabled:pointer-events-none"
        >
          Revert
        </button>
      </div>
      <div className="flex items-center justify-end gap-1.5">
        <button
          type="button"
          onClick={onCancel}
          className={`${size} text-ink-faint hover:text-ink-secondary transition-colors`}
        >
          Cancel
        </button>
        {/* Disabled controls keep their pointer events, so the title can say why. */}
        <button
          type="button"
          onClick={save}
          disabled={invalid !== null}
          title={invalid ?? undefined}
          className={`flex items-center gap-1 ${size} ${GATE_NEUTRAL} disabled:opacity-40 disabled:cursor-not-allowed`}
        >
          <Pencil size={icon} strokeWidth={1.75} />
          Save
        </button>
        {onApprove && (
          <button
            type="button"
            onClick={() => {
              if (!invalid) onApprove(edited)
            }}
            disabled={invalid !== null}
            title={invalid ? `Cannot approve yet. ${invalid}` : undefined}
            className={`flex items-center gap-1 ${size} ${GATE_APPROVE} disabled:opacity-40 disabled:cursor-not-allowed`}
          >
            <Check size={icon} strokeWidth={2.5} />
            Approve
          </button>
        )}
      </div>
    </div>
  )
}
