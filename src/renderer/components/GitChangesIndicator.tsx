import { FolderOpen } from 'lucide-react'
import { useAppStore } from '../stores'
import { Tooltip } from './Tooltip'

interface Props {
  terminalId: string
}

export function GitChangesIndicator({ terminalId }: Props) {
  const stat = useAppStore((s) => s.gitDiffStats.get(terminalId))
  const setDiffSidebar = useAppStore((s) => s.setDiffSidebarTerminalId)

  if (!stat || (stat.insertions === 0 && stat.deletions === 0)) return null

  const handleClick = (e: React.MouseEvent): void => {
    e.stopPropagation()
    setDiffSidebar(terminalId)
  }

  return (
    <button
      onClick={handleClick}
      className="flex items-center gap-1.5 px-1.5 py-0.5 text-[11px] font-mono
                 hover:bg-white/[0.06] rounded transition-colors cursor-pointer"
      title={`${stat.filesChanged} file${stat.filesChanged !== 1 ? 's' : ''} changed`}
    >
      {/* A measurement, not a verdict — a large deletion is not a failure. */}
      <span className="text-ink-secondary">+{stat.insertions}</span>
      <span className="text-ink-faint">-{stat.deletions}</span>
    </button>
  )
}

export function BrowseFilesButton({ terminalId }: Props) {
  const toggleFilesPane = useAppStore((s) => s.toggleFilesPane)

  const handleClick = (e: React.MouseEvent): void => {
    e.stopPropagation()
    toggleFilesPane(terminalId)
  }

  return (
    <Tooltip label="Browse files" position="top">
      <button
        onClick={handleClick}
        className="p-1 text-gray-600 hover:text-gray-300 rounded transition-colors"
        aria-label="Browse files"
      >
        <FolderOpen size={13} strokeWidth={1.5} />
      </button>
    </Tooltip>
  )
}
