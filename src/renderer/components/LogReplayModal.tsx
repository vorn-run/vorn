import { useRef, useEffect } from 'react'
import { X, Copy, CheckCircle } from 'lucide-react'
import { useState } from 'react'

interface Props {
  logs: string
  onClose: () => void
}

export function LogReplayModal({ logs, onClose }: Props) {
  const preRef = useRef<HTMLPreElement>(null)
  const [copied, setCopied] = useState(false)

  useEffect(() => {
    if (preRef.current) {
      preRef.current.scrollTop = preRef.current.scrollHeight
    }
  }, [logs])

  useEffect(() => {
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', handleKey)
    return () => window.removeEventListener('keydown', handleKey)
  }, [onClose])

  const handleCopy = async () => {
    await navigator.clipboard.writeText(logs)
    setCopied(true)
    setTimeout(() => setCopied(false), 2000)
  }

  const lineCount = logs.split('\n').length

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm">
      <div className="w-[90vw] max-w-[900px] h-[80vh] bg-surface-base border border-white/[0.1] rounded-xl shadow-2xl flex flex-col overflow-hidden">
        {/* Header */}
        <div className="flex items-center justify-between px-4 py-3 border-b border-white/[0.08] shrink-0">
          <div className="flex items-center gap-3">
            <span className="text-[13px] font-medium text-white">Full Output</span>
            <span className="text-[11px] text-gray-600">{lineCount} lines</span>
          </div>
          <div className="flex items-center gap-2">
            <button
              onClick={handleCopy}
              className="flex items-center gap-1.5 px-2.5 py-1 text-[11px] font-medium
                         bg-white/[0.04] hover:bg-white/[0.08] border border-white/[0.06]
                         rounded-md transition-colors text-gray-400 hover:text-gray-200"
            >
              {copied ? (
                <>
                  <CheckCircle size={12} strokeWidth={2} />
                  Copied
                </>
              ) : (
                <>
                  <Copy size={12} strokeWidth={2} />
                  Copy
                </>
              )}
            </button>
            <button
              onClick={onClose}
              className="text-gray-500 hover:text-white p-1 rounded-md transition-colors"
            >
              <X size={14} />
            </button>
          </div>
        </div>

        {/* Log content */}
        <pre
          ref={preRef}
          className="flex-1 overflow-auto p-4 text-[12px] text-gray-300 font-mono
                     whitespace-pre-wrap break-all leading-relaxed selection:bg-blue-500/30"
        >
          {logs}
        </pre>
      </div>
    </div>
  )
}
