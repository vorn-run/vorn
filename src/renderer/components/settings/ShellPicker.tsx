import { useCallback, useEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { Check, ChevronDown, PenLine } from 'lucide-react'
import type { InstalledShell } from '../../../shared/types'

/**
 * Choosing the shell a terminal runs.
 *
 * A free-text path was the previous control, which is fine if you already know
 * what is installed and where. The shells also differ in how much they can
 * report about a command — on Windows that is the difference between blocks
 * with exit status and blocks without — so each option carries that, and the
 * list is ordered best-first.
 *
 * A path can still be typed: something we did not detect is a legitimate
 * choice, and removing that would be a regression.
 */

/**
 * Each shell's own prompt sigil, set in a frame.
 *
 * Monograms rather than logos: the set stays consistent, nothing is fetched,
 * and every mark is legible at 20px. No colour — it is spent on status below,
 * which is the part worth looking at.
 */
const SIGILS: Record<InstalledShell['family'], string> = {
  zsh: '%',
  bash: '$',
  fish: '><>',
  powershell: 'PS',
  cmd: 'C:\\'
}

function ShellGlyph({ family }: { family: InstalledShell['family'] | null }) {
  const sigil = family ? SIGILS[family] : '·'
  return (
    <span
      aria-hidden
      className="flex h-6 w-6 shrink-0 items-center justify-center rounded-[4px]
                 border border-white/[0.12] font-mono text-[9px] leading-none text-gray-400"
    >
      {sigil}
    </span>
  )
}

/** Status, and the only place colour is spent here. */
function BlockSupport({ blocks }: { blocks: InstalledShell['blocks'] }) {
  if (blocks.level === 'full') {
    return <span className="text-[11px] text-gray-500">Full command blocks</span>
  }
  return (
    <span
      className={
        blocks.level === 'limited' ? 'text-[11px] text-amber-500/80' : 'text-[11px] text-gray-500'
      }
    >
      {blocks.limitation}
    </span>
  )
}

interface Props {
  value: string
  onChange: (shellPath: string) => void
}

export function ShellPicker({ value, onChange }: Props) {
  const [shells, setShells] = useState<InstalledShell[] | null>(null)
  const [open, setOpen] = useState(false)
  const [editing, setEditing] = useState(false)
  const [draft, setDraft] = useState(value)
  const triggerRef = useRef<HTMLButtonElement>(null)
  const menuRef = useRef<HTMLDivElement>(null)
  const [position, setPosition] = useState<{
    top?: number
    bottom?: number
    left: number
    width: number
  }>({ top: 0, left: 0, width: 0 })

  useEffect(() => {
    let cancelled = false
    void window.api
      .listInstalledShells?.()
      .then((found) => {
        if (!cancelled) setShells(found)
      })
      .catch(() => {
        // Detection is a convenience; typing a path still works without it.
        if (!cancelled) setShells([])
      })
    return () => {
      cancelled = true
    }
  }, [])

  const current = shells?.find((s) => s.path === value) ?? null

  const openMenu = useCallback(() => {
    if (open) {
      setOpen(false)
      return
    }
    const rect = triggerRef.current?.getBoundingClientRect()
    if (rect) {
      // Settings scrolls, so the trigger is often near the bottom. Anchoring a
      // flipped menu by its bottom edge means its height never has to be known.
      const estimated = ((shells?.length ?? 3) + 1) * 56 + 8
      const flipUp =
        rect.bottom + 4 + estimated > window.innerHeight - 8 && rect.top - 4 - estimated > 8
      setPosition({
        top: flipUp ? undefined : rect.bottom + 4,
        bottom: flipUp ? window.innerHeight - rect.top + 4 : undefined,
        left: rect.left,
        width: rect.width
      })
    }
    setOpen(true)
  }, [open, shells])

  useEffect(() => {
    if (!open) return
    const onClick = (e: MouseEvent): void => {
      const target = e.target as Node
      if (triggerRef.current?.contains(target)) return
      if (menuRef.current && !menuRef.current.contains(target)) setOpen(false)
    }
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === 'Escape') setOpen(false)
    }
    document.addEventListener('mousedown', onClick)
    document.addEventListener('keydown', onKey)
    return () => {
      document.removeEventListener('mousedown', onClick)
      document.removeEventListener('keydown', onKey)
    }
  }, [open])

  const commitDraft = (): void => {
    const next = draft.trim()
    if (next && next !== value) onChange(next)
    setEditing(false)
  }

  if (editing) {
    return (
      <div className="flex w-72 items-center gap-2">
        <input
          autoFocus
          type="text"
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={commitDraft}
          onKeyDown={(e) => {
            if (e.key === 'Enter') commitDraft()
            if (e.key === 'Escape') {
              setDraft(value)
              setEditing(false)
            }
          }}
          placeholder="Path to a shell"
          className="w-full rounded-[4px] border border-white/[0.12] bg-white/[0.04] px-2 py-1.5
                     font-mono text-[12px] text-gray-200 placeholder:text-gray-600
                     focus:border-white/25 focus:outline-none"
        />
      </div>
    )
  }

  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        onClick={openMenu}
        aria-haspopup="listbox"
        aria-expanded={open}
        className="flex w-72 items-center gap-2 rounded-[4px] border border-white/[0.12]
                   px-2 py-1.5 text-left transition-colors hover:border-white/25"
      >
        <ShellGlyph family={current?.family ?? null} />
        <span className="min-w-0 flex-1">
          <span className="block truncate text-[13px] text-gray-200">
            {current?.name ?? value.split(/[\\/]/).pop() ?? 'Select a shell'}
          </span>
          <span className="block truncate font-mono text-[11px] text-gray-600">{value}</span>
        </span>
        <ChevronDown size={12} className="shrink-0 text-gray-500" />
      </button>

      {createPortal(
        open ? (
          <div
            ref={menuRef}
            role="listbox"
            className="fixed z-[150] rounded-[4px] border border-white/[0.12] py-1"
            style={{
              top: position.top,
              bottom: position.bottom,
              left: position.left,
              minWidth: Math.max(288, position.width),
              background: '#1e1e22'
            }}
          >
            {shells === null && (
              <div className="px-3 py-2 text-[12px] text-gray-500">Looking for shells…</div>
            )}
            {shells?.length === 0 && (
              <div className="px-3 py-2 text-[12px] text-gray-500">
                No shells detected. Enter a path instead.
              </div>
            )}
            {shells?.map((shell) => (
              <button
                key={shell.path}
                type="button"
                role="option"
                aria-selected={shell.path === value}
                onClick={() => {
                  setOpen(false)
                  if (shell.path !== value) onChange(shell.path)
                }}
                className="flex w-full items-start gap-2 px-3 py-2 text-left transition-colors
                           hover:bg-white/[0.06]"
              >
                <span className="mt-[1px]">
                  <ShellGlyph family={shell.family} />
                </span>
                <span className="min-w-0 flex-1">
                  <span className="flex items-baseline gap-1.5">
                    <span className="truncate text-[13px] text-gray-200">{shell.name}</span>
                    {shell.version && (
                      <span className="shrink-0 font-mono text-[11px] text-gray-600">
                        {shell.version}
                      </span>
                    )}
                  </span>
                  <span className="block truncate font-mono text-[11px] text-gray-600">
                    {shell.path}
                  </span>
                  <span className="mt-0.5 block">
                    <BlockSupport blocks={shell.blocks} />
                  </span>
                </span>
                {shell.path === value && (
                  <Check size={13} className="mt-1 shrink-0 text-gray-400" />
                )}
              </button>
            ))}

            <div className="my-1 border-t border-white/[0.06]" />
            <button
              type="button"
              onClick={() => {
                setOpen(false)
                setDraft(value)
                setEditing(true)
              }}
              className="flex w-full items-center gap-2 px-3 py-2 text-left transition-colors
                         hover:bg-white/[0.06]"
            >
              <PenLine size={13} className="shrink-0 text-gray-500" />
              <span className="text-[12px] text-gray-400">Enter a path…</span>
            </button>
          </div>
        ) : null,
        document.body
      )}
    </>
  )
}
