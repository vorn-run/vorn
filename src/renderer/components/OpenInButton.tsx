import { useState, useEffect, useRef } from 'react'
import { createPortal } from 'react-dom'
import { ChevronDown } from 'lucide-react'

import vscodeSvg from '../assets/icons/vscode.svg?raw'
import cursorSvg from '../assets/icons/cursor.svg?raw'
import windsurfSvg from '../assets/icons/windsurf.svg?raw'
import zedSvg from '../assets/icons/zed.svg?raw'
import sublimeSvg from '../assets/icons/sublime.svg?raw'
import webstormSvg from '../assets/icons/webstorm.svg?raw'
import intellijSvg from '../assets/icons/intellij.svg?raw'
import xcodeSvg from '../assets/icons/xcode.svg?raw'
import terminalSvg from '../assets/icons/terminal.svg?raw'
import finderSvg from '../assets/icons/finder.svg?raw'

interface DetectedIDE {
  id: string
  name: string
  command: string
}

const IDE_ICONS: Record<string, string> = {
  vscode: vscodeSvg,
  'vscode-insiders': vscodeSvg,
  cursor: cursorSvg,
  windsurf: windsurfSvg,
  zed: zedSvg,
  sublime: sublimeSvg,
  webstorm: webstormSvg,
  intellij: intellijSvg,
  xcode: xcodeSvg,
  terminal: terminalSvg,
  finder: finderSvg
}

function IDEIcon({ ideId, size = 14 }: { ideId: string; size?: number }) {
  const svg = IDE_ICONS[ideId]
  if (!svg) return null
  return (
    <span
      className="ide-icon-wrap inline-flex items-center justify-center shrink-0"
      style={{ width: size, height: size }}
      dangerouslySetInnerHTML={{ __html: svg }}
    />
  )
}

let ideCache: DetectedIDE[] | null = null
let ideCachePromise: Promise<DetectedIDE[]> | null = null

function loadIDEs(): Promise<DetectedIDE[]> {
  if (ideCache) return Promise.resolve(ideCache)
  if (!ideCachePromise) {
    ideCachePromise = window.api.detectIDEs().then(
      (detected) => {
        ideCache = detected
        return detected
      },
      (err) => {
        ideCachePromise = null
        throw err
      }
    )
  }
  return ideCachePromise
}

interface Props {
  projectPath: string
  direction?: 'up' | 'down'
}

const MENU_WIDTH = 180

export function OpenInButton({ projectPath, direction = 'down' }: Props) {
  const [ides, setIdes] = useState<DetectedIDE[]>(ideCache || [])
  const [isOpen, setIsOpen] = useState(false)
  const [menuPos, setMenuPos] = useState<{ top: number; left: number } | null>(null)
  const menuRef = useRef<HTMLDivElement>(null)
  const anchorRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (ideCache) return
    loadIDEs().then(setIdes, () => {
      /* detection failed; UI stays hidden until a later retry */
    })
  }, [])

  useEffect(() => {
    if (!isOpen) return
    const handleClickOutside = (e: MouseEvent) => {
      const target = e.target as Node
      if (menuRef.current?.contains(target)) return
      if (anchorRef.current?.contains(target)) return
      setIsOpen(false)
    }
    document.addEventListener('mousedown', handleClickOutside)
    return () => document.removeEventListener('mousedown', handleClickOutside)
  }, [isOpen])

  if (ides.length === 0) return null

  const defaultIDE = ides[0]

  const handleOpen = (e: React.MouseEvent) => {
    e.stopPropagation()
    window.api.openInIDE(defaultIDE.id, projectPath)
  }

  const handleToggle = (e: React.MouseEvent) => {
    e.stopPropagation()
    if (isOpen) {
      setIsOpen(false)
      return
    }
    const rect = anchorRef.current?.getBoundingClientRect()
    if (rect) {
      const estimatedHeight = 32 + ides.length * 30
      const left = Math.max(
        8,
        Math.min(rect.right - MENU_WIDTH, window.innerWidth - MENU_WIDTH - 8)
      )
      const top = direction === 'up' ? Math.max(8, rect.top - estimatedHeight - 4) : rect.bottom + 4
      setMenuPos({ top, left })
    }
    setIsOpen(true)
  }

  const handleSelect = (ide: DetectedIDE, e: React.MouseEvent) => {
    e.stopPropagation()
    window.api.openInIDE(ide.id, projectPath)
    setIsOpen(false)
  }

  return (
    <div className="relative" ref={anchorRef}>
      <div className="flex items-center">
        <button
          onClick={handleOpen}
          className="flex items-center gap-1 px-1 py-0.5 text-gray-500 hover:text-gray-300
                     hover:bg-white/[0.06] rounded-l transition-colors"
          title={`Open in ${defaultIDE.name}`}
        >
          <IDEIcon ideId={defaultIDE.id} size={11} />
          <span className="text-[10px]">Open</span>
        </button>
        <button
          onClick={handleToggle}
          aria-label="Choose IDE"
          className="flex items-center px-0.5 py-0.5 text-gray-500 hover:text-gray-300
                     hover:bg-white/[0.06] rounded-r transition-colors self-stretch"
        >
          <ChevronDown size={9} strokeWidth={2} />
        </button>
      </div>

      {isOpen &&
        menuPos &&
        createPortal(
          <div
            ref={menuRef}
            className="fixed z-[150] py-1 border border-white/[0.08] rounded-lg shadow-xl"
            style={{
              background: 'var(--color-surface-overlay)',
              top: menuPos.top,
              left: menuPos.left,
              width: MENU_WIDTH
            }}
          >
            <div className="px-3 py-1.5 text-[11px] text-gray-500 font-medium">Open in</div>
            {ides.map((ide) => (
              <button
                key={ide.id}
                onClick={(e) => handleSelect(ide, e)}
                className="w-full px-3 py-1.5 text-left text-[13px] text-gray-300 hover:text-white
                           hover:bg-white/[0.06] flex items-center gap-2.5 transition-colors"
              >
                <IDEIcon ideId={ide.id} size={14} />
                {ide.name}
              </button>
            ))}
          </div>,
          document.body
        )}
    </div>
  )
}
