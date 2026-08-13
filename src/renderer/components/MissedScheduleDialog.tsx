import { useState, useEffect } from 'react'
import { motion, AnimatePresence } from 'framer-motion'
import { useAppStore } from '../stores'
import { executeWorkflow } from '../lib/workflow-execution'
import { Clock, AlertTriangle } from 'lucide-react'

interface MissedItem {
  workflow: { id: string; name: string }
  scheduledFor: string
}

export function MissedScheduleDialog() {
  const [missed, setMissed] = useState<MissedItem[]>([])
  const [selected, setSelected] = useState<Set<string>>(new Set())
  const config = useAppStore((s) => s.config)

  useEffect(() => {
    const remove = window.api.onSchedulerMissed((items) => {
      setMissed(items)
      setSelected(new Set(items.map((i) => i.workflow.id)))
    })
    return remove
  }, [])

  if (missed.length === 0) return null

  const handleRunSelected = async () => {
    for (const item of missed) {
      if (!selected.has(item.workflow.id)) continue
      const wf = config?.workflows?.find((w) => w.id === item.workflow.id)
      if (!wf) continue
      await executeWorkflow(wf)
    }
    setMissed([])
  }

  const handleSkip = () => setMissed([])

  const toggleSelect = (id: string) => {
    setSelected((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }

  return (
    <AnimatePresence>
      <motion.div
        className="fixed inset-0 bg-black/50 z-[70]"
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        exit={{ opacity: 0 }}
      />
      <motion.div
        className="fixed top-1/2 left-1/2 z-[70] w-[420px] border border-white/[0.08]
                   rounded-xl shadow-2xl overflow-hidden flex flex-col"
        style={{ background: 'var(--color-surface-overlay)' }}
        initial={{ opacity: 0, scale: 0.95, x: '-50%', y: '-50%' }}
        animate={{ opacity: 1, scale: 1, x: '-50%', y: '-50%' }}
        exit={{ opacity: 0, scale: 0.95, x: '-50%', y: '-50%' }}
        transition={{ type: 'spring', stiffness: 400, damping: 30 }}
      >
        <div className="px-5 py-4 border-b border-white/[0.06]">
          <div className="flex items-center gap-2">
            <AlertTriangle size={16} className="text-amber-400" />
            <h2 className="text-base font-medium text-white">Missed Schedules</h2>
          </div>
          <p className="text-sm text-gray-500 mt-1">
            These workflows were scheduled while Vorn was closed.
          </p>
        </div>

        <div className="p-4 space-y-2">
          {missed.map((item) => (
            <label
              key={item.workflow.id}
              className="flex items-center gap-3 p-3 bg-white/[0.03] border border-white/[0.06]
                         rounded-lg cursor-pointer hover:bg-white/[0.05] transition-colors"
            >
              <input
                type="checkbox"
                checked={selected.has(item.workflow.id)}
                onChange={() => toggleSelect(item.workflow.id)}
                className="rounded border-gray-600 bg-white/[0.05] text-blue-500
                           focus:ring-0 focus:ring-offset-0"
              />
              <div className="flex-1 min-w-0">
                <span className="text-sm text-gray-200 block">{item.workflow.name}</span>
                <span className="text-[11px] text-gray-500 flex items-center gap-1 mt-0.5">
                  <Clock size={10} />
                  Scheduled for {new Date(item.scheduledFor).toLocaleString()}
                </span>
              </div>
            </label>
          ))}
        </div>

        <div className="px-5 py-3 border-t border-white/[0.06] flex justify-end gap-3">
          <button
            onClick={handleSkip}
            className="px-4 py-2 text-sm text-gray-400 hover:text-gray-200
                       bg-white/[0.04] hover:bg-white/[0.08] rounded-lg transition-colors"
          >
            Skip All
          </button>
          <button
            onClick={handleRunSelected}
            disabled={selected.size === 0}
            className="px-4 py-2 text-sm font-medium text-white
                       bg-white/[0.1] hover:bg-white/[0.15]
                       disabled:opacity-30 disabled:cursor-not-allowed
                       rounded-lg transition-colors"
          >
            Run Selected ({selected.size})
          </button>
        </div>
      </motion.div>
    </AnimatePresence>
  )
}
