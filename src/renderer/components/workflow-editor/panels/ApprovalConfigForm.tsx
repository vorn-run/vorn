import { ApprovalConfig } from '../../../../shared/types'

interface Props {
  config: ApprovalConfig
  onChange: (config: ApprovalConfig) => void
}

export function ApprovalConfigForm({ config, onChange }: Props) {
  const timeoutSeconds =
    config.timeoutMs && config.timeoutMs > 0 ? Math.round(config.timeoutMs / 1000) : ''

  return (
    <div className="space-y-5">
      <div>
        <label className="text-[13px] text-gray-400 font-medium block mb-2">Message</label>
        <textarea
          value={config.message || ''}
          onChange={(e) => onChange({ ...config, message: e.target.value })}
          placeholder="Shown next to the Approve / Reject buttons"
          rows={3}
          className="w-full px-3 py-2 bg-white/[0.03] border border-white/[0.08] rounded-md
                     text-[13px] text-gray-200 placeholder:text-gray-600
                     focus:outline-none focus:border-white/[0.2] resize-none"
        />
      </div>

      <div>
        <label className="text-[13px] text-gray-400 font-medium block mb-2">
          Timeout <span className="text-gray-600">(seconds, optional)</span>
        </label>
        <input
          type="number"
          min={0}
          value={timeoutSeconds}
          onChange={(e) => {
            const raw = e.target.value.trim()
            const secs = Number(raw)
            const valid = raw !== '' && Number.isFinite(secs) && secs > 0
            onChange({ ...config, timeoutMs: valid ? Math.round(secs * 1000) : undefined })
          }}
          placeholder="Leave blank to wait forever"
          className="w-full px-3 py-2 bg-white/[0.03] border border-white/[0.08] rounded-md
                     text-[13px] text-gray-200 placeholder:text-gray-600
                     focus:outline-none focus:border-white/[0.2]"
        />
        <div className="mt-1.5 text-[11px] text-gray-500">
          Auto-rejects the gate if nobody approves in time.
        </div>
      </div>
    </div>
  )
}
