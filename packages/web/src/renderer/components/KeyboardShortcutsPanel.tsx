import { motion } from 'framer-motion'
import { useAppStore } from '../stores'
import { SHORTCUTS, SHORTCUT_CATEGORIES } from '../lib/keyboard-shortcuts'

export function KeyboardShortcutsPanel() {
  const close = () => useAppStore.getState().setShortcutsPanelOpen(false)

  // Group shortcuts by category, only those with a description
  const grouped = SHORTCUT_CATEGORIES.map(({ key, label }) => ({
    label,
    shortcuts: SHORTCUTS.filter((s) => s.category === key && s.description)
  })).filter((g) => g.shortcuts.length > 0)

  return (
    <>
      <motion.div
        className="fixed inset-0 bg-black/40 z-[60]"
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        exit={{ opacity: 0 }}
        onClick={close}
      />

      <motion.div
        className="fixed top-[15%] left-1/2 z-[60] w-[520px] border border-white/[0.08]
                   rounded-xl shadow-2xl overflow-hidden flex flex-col"
        style={{ background: 'var(--color-surface-overlay)' }}
        initial={{ opacity: 0, scale: 0.98, x: '-50%', y: -8 }}
        animate={{ opacity: 1, scale: 1, x: '-50%', y: 0 }}
        exit={{ opacity: 0, scale: 0.98, x: '-50%', y: -8 }}
        transition={{ duration: 0.15 }}
      >
        {/* Header */}
        <div className="px-5 py-4 border-b border-white/[0.06] flex items-center justify-between">
          <h2 className="text-sm font-medium text-white">Keyboard Shortcuts</h2>
          <kbd
            className="text-[10px] text-gray-600 bg-white/[0.04]
                          border border-white/[0.06] px-1.5 py-0.5 rounded font-mono"
          >
            ESC
          </kbd>
        </div>

        {/* Shortcut categories */}
        <div className="px-5 py-4 space-y-5 max-h-[400px] overflow-auto">
          {grouped.map((group) => (
            <div key={group.label}>
              <div className="text-[11px] font-medium text-gray-500 uppercase tracking-wider mb-2">
                {group.label}
              </div>
              <div className="space-y-0.5">
                {group.shortcuts.map((shortcut) => (
                  <div
                    key={shortcut.id}
                    className="flex items-center justify-between py-1.5 px-2 rounded-md
                               hover:bg-white/[0.03] transition-colors"
                  >
                    <span className="text-[13px] text-gray-300">{shortcut.description}</span>
                    <kbd
                      className="text-[11px] text-gray-400 bg-white/[0.04]
                                    border border-white/[0.06] px-2 py-0.5 rounded font-mono shrink-0 ml-4"
                    >
                      {shortcut.display}
                    </kbd>
                  </div>
                ))}
              </div>
            </div>
          ))}

          {/* Extra tips */}
          <div>
            <div className="text-[11px] font-medium text-gray-500 uppercase tracking-wider mb-2">
              Tips
            </div>
            <div className="space-y-0.5">
              {[['Double-click card title', 'Rename inline']].map(([key, desc]) => (
                <div
                  key={key}
                  className="flex items-center justify-between py-1.5 px-2 rounded-md
                             hover:bg-white/[0.03] transition-colors"
                >
                  <span className="text-[13px] text-gray-300">{desc}</span>
                  <span className="text-[11px] text-gray-500 font-mono shrink-0 ml-4">{key}</span>
                </div>
              ))}
            </div>
          </div>

          {/* Welcome Guide link */}
          <div className="pt-3 border-t border-white/[0.06]">
            <button
              onClick={() => {
                useAppStore.getState().setShortcutsPanelOpen(false)
                useAppStore.getState().setOnboardingOpen(true)
              }}
              className="w-full flex items-center justify-between py-1.5 px-2 rounded-md
                         hover:bg-white/[0.06] transition-colors text-left"
            >
              <span className="text-[13px] text-bronzo">Show Welcome Guide</span>
              <span className="text-[11px] text-gray-500">Feature tour</span>
            </button>
          </div>
        </div>
      </motion.div>
    </>
  )
}
