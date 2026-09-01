import { useEffect, useState } from 'react'
import { Plus, X } from 'lucide-react'
import type { HttpRequestConfig, SourceConnection, TriggerConfig } from '../../../../shared/types'
import { SelectPicker } from '../../SelectPicker'
import {
  getAvailableContextVars,
  StepVariableGroup,
  TemplateVariable
} from '../../../lib/template-vars'
import { VariableAutocomplete } from './VariableAutocomplete'

const METHODS = ['GET', 'POST', 'PUT', 'PATCH', 'DELETE'].map((m) => ({ value: m, label: m }))

interface Props {
  config: HttpRequestConfig
  onChange: (config: HttpRequestConfig) => void
  triggerType?: TriggerConfig['triggerType']
  isContextualTrigger?: boolean
  inputVars?: TemplateVariable[]
  stepGroups?: StepVariableGroup[]
}

export function HttpRequestConfigForm({
  config,
  onChange,
  triggerType,
  isContextualTrigger = false,
  inputVars = [],
  stepGroups = []
}: Props) {
  const [profiles, setProfiles] = useState<SourceConnection[]>([])

  useEffect(() => {
    window.api.listConnections().then((conns) => {
      setProfiles(conns.filter((c) => c.connectorId === 'http'))
    })
  }, [])

  const contextVars = [
    ...getAvailableContextVars({ triggerType, isContextualTrigger }),
    ...inputVars
  ]
  const headerEntries = Object.entries(config.headers ?? {})

  const setHeaders = (entries: [string, string][]) => {
    onChange({ ...config, headers: Object.fromEntries(entries) })
  }

  return (
    <div className="space-y-5">
      <div className="flex gap-2">
        <div className="w-[110px]">
          <label className="text-[13px] text-gray-400 font-medium block mb-2">Method</label>
          <SelectPicker
            value={config.method}
            options={METHODS}
            onChange={(v) => onChange({ ...config, method: v as HttpRequestConfig['method'] })}
            variant="form"
          />
        </div>
        <div className="flex-1 min-w-0">
          <label className="text-[13px] text-gray-400 font-medium block mb-2">URL</label>
          <VariableAutocomplete
            value={config.url}
            onChange={(url) => onChange({ ...config, url })}
            placeholder={config.profileConnectionId ? '/v1/items' : 'https://api.example.com/items'}
            rows={1}
            stepGroups={stepGroups}
            contextVars={contextVars}
            mono
          />
        </div>
      </div>

      <div>
        <label className="text-[13px] text-gray-400 font-medium block mb-2">Auth profile</label>
        <SelectPicker
          value={config.profileConnectionId ?? ''}
          options={[
            { value: '', label: 'None' },
            ...profiles.map((p) => ({ value: p.id, label: p.name }))
          ]}
          onChange={(v) => onChange({ ...config, profileConnectionId: v || undefined })}
          variant="form"
        />
        <p className="text-[11px] text-gray-600 mt-1.5">
          A profile adds its base URL and auth to the request on the server, so its secret never
          enters this workflow.
        </p>
      </div>

      <div>
        <label className="text-[13px] text-gray-400 font-medium block mb-2">Headers</label>
        <div className="space-y-1.5">
          {headerEntries.map(([name, value], i) => (
            <div key={i} className="flex items-center gap-1.5">
              <input
                value={name}
                onChange={(e) => {
                  const next = [...headerEntries] as [string, string][]
                  next[i] = [e.target.value, value]
                  setHeaders(next)
                }}
                placeholder="Name"
                className="w-[140px] bg-white/[0.03] border border-white/[0.08] rounded px-2 py-1.5 text-[12px] font-mono text-white placeholder:text-gray-600 focus:outline-none focus:border-white/[0.2]"
              />
              <div className="flex-1 min-w-0">
                <VariableAutocomplete
                  value={value}
                  onChange={(v) => {
                    const next = [...headerEntries] as [string, string][]
                    next[i] = [name, v]
                    setHeaders(next)
                  }}
                  placeholder="Value"
                  rows={1}
                  stepGroups={stepGroups}
                  contextVars={contextVars}
                  mono
                />
              </div>
              <button
                aria-label="Remove header"
                onClick={() =>
                  setHeaders(headerEntries.filter((_, j) => j !== i) as [string, string][])
                }
                className="p-1 rounded text-gray-500 hover:text-white hover:bg-white/[0.06] transition-colors"
              >
                <X size={12} />
              </button>
            </div>
          ))}
          <button
            onClick={() => setHeaders([...headerEntries, ['', '']] as [string, string][])}
            className="flex items-center gap-1 text-[11px] text-gray-500 hover:text-gray-300 transition-colors"
          >
            <Plus size={11} />
            Add header
          </button>
        </div>
      </div>

      <div>
        <label className="text-[13px] text-gray-400 font-medium block mb-2">Body</label>
        <VariableAutocomplete
          value={config.body}
          onChange={(body) => onChange({ ...config, body })}
          placeholder='{"name": "{{trigger.body.name}}"}'
          rows={6}
          stepGroups={stepGroups}
          contextVars={contextVars}
          mono
        />
        <p className="text-[11px] text-gray-600 mt-1.5">Sent as-is. GET requests send no body.</p>
      </div>
    </div>
  )
}
