import { useCallback, useLayoutEffect, useRef, useState, useSyncExternalStore } from 'react'
import { blockToText, type BlockRow, type StyledRun } from '../lib/block-render'
import { colorToCss, getBlockLog, onBlockLogChange, type LoggedBlock } from '../lib/block-log'
import { formatDuration, shortenCwd } from '../lib/command-blocks'
import { Check, Copy } from 'lucide-react'
import { TERMINAL_BACKGROUND } from '../lib/surface'

/**
 * Finished commands, drawn as real elements.
 *
 * Each block is an ordinary container, so its padding, its boundary and its
 * hover state are CSS rather than something approximated inside the character
 * grid — and nothing has to be printed into the user's shell to fake them.
 */

interface Props {
  terminalId: string
  className?: string
}

const FG_DEFAULT = '#d4d4d8'
const BG_DEFAULT = 'transparent'

function runStyle(run: StyledRun): React.CSSProperties {
  // Inverse is a draw-time swap in a terminal, so resolve it here rather than
  // carrying it into the DOM.
  const fg = colorToCss(run.fg, FG_DEFAULT)
  const bg = colorToCss(run.bg, BG_DEFAULT)
  return {
    color: run.inverse ? (bg === 'transparent' ? TERMINAL_BACKGROUND : bg) : fg,
    background: run.inverse ? fg : bg,
    fontWeight: run.bold ? 600 : undefined,
    fontStyle: run.italic ? 'italic' : undefined,
    opacity: run.dim ? 0.6 : undefined,
    textDecoration:
      run.underline && run.strikethrough
        ? 'underline line-through'
        : run.underline
          ? 'underline'
          : run.strikethrough
            ? 'line-through'
            : undefined
  }
}

function Row({ row }: { row: BlockRow }) {
  if (row.runs.length === 0) return <div>&nbsp;</div>
  return (
    <div>
      {row.runs.map((run, i) => (
        <span key={i} style={runStyle(run)}>
          {run.text}
        </span>
      ))}
    </div>
  )
}

function Block({ block }: { block: LoggedBlock }) {
  const [copied, setCopied] = useState(false)
  const ok = block.exitCode === 0
  const dir = shortenCwd(block.cwd)

  const copy = useCallback(() => {
    void navigator.clipboard.writeText(blockToText(block.rows)).then(() => {
      setCopied(true)
      setTimeout(() => setCopied(false), 1200)
    })
  }, [block.rows])

  return (
    <div
      className="group/blk relative border-t border-white/[0.06] first:border-t-0
                 py-2 pl-3 pr-3 transition-colors hover:bg-white/[0.03]"
    >
      {/* Status rail. Spans the block exactly, so it cannot appear to belong
          to whatever sits below it. */}
      <div
        className="absolute left-0 top-0 bottom-0 w-[2px] rounded-[2px]"
        style={{ background: ok ? 'transparent' : '#f87171' }}
      />

      {/* Positioned rather than laid out beside the command: as a flex item it
          could be pushed onto its own line, which made the block taller than
          its content and read as belonging to the next command. */}
      <div className="absolute top-2 right-3 flex items-center gap-2 text-[10px] tabular-nums leading-[19px] whitespace-nowrap">
        {dir && <span className="font-mono text-white/25">{dir}</span>}
        {!ok && <span className="font-mono text-red-400">exit {block.exitCode}</span>}
        <span className="font-mono text-white/25">{formatDuration(block.durationMs)}</span>
        <button
          type="button"
          onClick={copy}
          aria-label="Copy output"
          className="opacity-0 group-hover/blk:opacity-100 transition-opacity
                     text-gray-500 hover:text-gray-200"
        >
          {copied ? <Check size={11} /> : <Copy size={11} />}
        </button>
      </div>

      {/* Right padding keeps a long command clear of the metadata above it. */}
      <div className="pr-40 font-mono text-[13px] leading-[19px] font-semibold text-gray-100 whitespace-pre-wrap break-words">
        {block.command !== null ? (
          block.command
        ) : (
          // cmd.exe cannot report the command text, so it stays where the user
          // typed it — the block's first row. Titling these "(command)" would
          // print a placeholder above the real thing.
          <Row row={block.rows[0]} />
        )}
      </div>

      {block.rows.length > 1 && (
        <div className="font-mono text-[13px] leading-[19px] text-gray-300 whitespace-pre-wrap break-words">
          {block.rows.slice(1).map((row, i) => (
            <Row key={i} row={row} />
          ))}
        </div>
      )}
    </div>
  )
}

export function BlockLog({ terminalId, className }: Props) {
  const blocks = useSyncExternalStore(
    useCallback((cb: () => void) => onBlockLogChange(terminalId, cb), [terminalId]),
    useCallback(() => getBlockLog(terminalId), [terminalId])
  )
  const endRef = useRef<HTMLDivElement>(null)
  const scrollRef = useRef<HTMLDivElement>(null)
  const pinnedRef = useRef(true)

  // Follow the tail only when the user is already there, so reading history
  // is not interrupted by a command finishing.
  const onScroll = useCallback(() => {
    const el = scrollRef.current
    if (!el) return
    pinnedRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24
  }, [])

  useLayoutEffect(() => {
    if (pinnedRef.current) endRef.current?.scrollIntoView({ block: 'end' })
  }, [blocks])

  if (blocks.length === 0) return null

  return (
    <div
      ref={scrollRef}
      onScroll={onScroll}
      className={`overflow-y-auto ${className ?? ''}`}
      style={{ background: 'var(--color-surface-sunken)' }}
    >
      <div className="flex flex-col">
        {blocks.map((b) => (
          <Block key={b.id} block={b} />
        ))}
      </div>
      <div ref={endRef} />
    </div>
  )
}
