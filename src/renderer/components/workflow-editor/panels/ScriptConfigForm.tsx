import { useEffect, useState } from 'react'
import { motion, AnimatePresence } from 'framer-motion'
import { Terminal, FileCode, Braces, ChevronRight, Settings2 } from 'lucide-react'
import { ScriptConfig, TriggerConfig } from '../../../../shared/types'
import { useAppStore } from '../../../stores'
import {
  StepVariableGroup,
  CONTEXT_REF,
  getAvailableContextVars,
  isContextRef
} from '../../../lib/template-vars'
import { VariableAutocomplete } from './VariableAutocomplete'
import { ProjectPicker } from '../../ProjectPicker'
import { SelectPicker } from '../../SelectPicker'

interface Props {
  config: ScriptConfig
  onChange: (config: ScriptConfig) => void
  triggerType?: TriggerConfig['triggerType']
  isContextualTrigger?: boolean
  stepGroups?: StepVariableGroup[]
}

const EMPTY_PROJECTS: import('../../../../shared/types').ProjectConfig[] = []

const SCRIPT_TYPES = [
  { value: 'bash', label: 'Bash', icon: <Terminal size={12} className="text-gray-400" /> },
  {
    value: 'powershell',
    label: 'PowerShell',
    icon: <Terminal size={12} className="text-gray-400" />
  },
  { value: 'python', label: 'Python', icon: <FileCode size={12} className="text-gray-400" /> },
  { value: 'node', label: 'Node', icon: <Braces size={12} className="text-gray-400" /> }
]

export function ScriptConfigForm({
  config,
  onChange,
  triggerType,
  isContextualTrigger = false,
  stepGroups = []
}: Props) {
  const [advancedOpen, setAdvancedOpen] = useState(!!config.args?.length)
  const projects = useAppStore((s) => s.config?.projects ?? EMPTY_PROJECTS)
  const isTaskTrigger = triggerType === 'taskCreated' || triggerType === 'taskStatusChanged'
  const hasTemplateVars = stepGroups.length > 0 || isTaskTrigger || isContextualTrigger
  const contextVars = getAvailableContextVars({ triggerType, isContextualTrigger })
  const cwdIsFromContext =
    isContextRef(config.cwd) || isContextRef(config.projectName) || isContextRef(config.projectPath)

  // The runtime resolver returns empty strings for `{{context.*}}` when
  // there's no source, so leaving sentinels in place after toggling off
  // would silently launch with empty cwd.
  useEffect(() => {
    if (isContextualTrigger || !cwdIsFromContext) return
    onChange({ ...config, cwd: undefined, projectName: undefined, projectPath: undefined })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isContextualTrigger])

  return (
    <div className="space-y-5">
      <div>
        <label className="text-[13px] text-gray-400 font-medium block mb-2">Type</label>
        <SelectPicker
          value={config.scriptType}
          options={SCRIPT_TYPES}
          onChange={(v) => onChange({ ...config, scriptType: v as ScriptConfig['scriptType'] })}
          variant="form"
        />
      </div>

      <div>
        <label className="text-[13px] text-gray-400 font-medium block mb-2">
          Working Directory
        </label>
        <ProjectPicker
          currentProject={cwdIsFromContext ? '' : config.projectName || ''}
          projects={projects}
          onChange={(name) => {
            if (!name) {
              const { projectName: _pn, projectPath: _pp, cwd: _cwd, ...rest } = config
              onChange(rest as ScriptConfig)
            } else {
              const proj = projects.find((p) => p.name === name)
              if (proj) {
                onChange({
                  ...config,
                  projectName: proj.name,
                  projectPath: proj.path,
                  cwd: proj.path
                })
              }
            }
          }}
          variant="form"
          allowNone
          allowFromContext={isContextualTrigger}
          isFromContext={cwdIsFromContext}
          onSelectFromContext={() =>
            onChange({
              ...config,
              projectName: CONTEXT_REF.projectName,
              projectPath: CONTEXT_REF.projectPath,
              cwd: CONTEXT_REF.cwd
            })
          }
        />
      </div>

      <div>
        <label className="text-[13px] text-gray-400 font-medium block mb-2">Script</label>
        <VariableAutocomplete
          value={config.scriptContent || ''}
          onChange={(val) => onChange({ ...config, scriptContent: val })}
          placeholder={`Enter ${config.scriptType} script...`}
          rows={10}
          stepGroups={stepGroups}
          contextVars={contextVars}
          mono
        />
        {hasTemplateVars && (
          <p className="text-[11px] text-gray-500 mt-1">
            Type {'{{'} to insert step outputs or trigger variables
          </p>
        )}
      </div>

      <div className="border-t border-white/[0.06] pt-4">
        <button
          onClick={() => setAdvancedOpen(!advancedOpen)}
          className="flex items-center gap-1.5 text-[11px] text-gray-500 hover:text-gray-400
                     transition-colors uppercase tracking-wider font-medium w-full"
        >
          <ChevronRight
            size={12}
            className={`transition-transform duration-200 ${advancedOpen ? 'rotate-90' : ''}`}
          />
          <Settings2 size={11} />
          Advanced
        </button>

        <AnimatePresence initial={false}>
          {advancedOpen && (
            <motion.div
              initial={{ height: 0, opacity: 0 }}
              animate={{ height: 'auto', opacity: 1 }}
              exit={{ height: 0, opacity: 0 }}
              transition={{ duration: 0.2, ease: 'easeInOut' }}
              className="overflow-hidden"
            >
              <div className="space-y-4 pt-3">
                <div>
                  <label className="text-[13px] text-gray-400 font-medium block mb-2">
                    Arguments
                  </label>
                  <input
                    type="text"
                    value={(config.args || []).join(' ')}
                    onChange={(e) =>
                      onChange({ ...config, args: e.target.value.split(' ').filter(Boolean) })
                    }
                    placeholder="arg1 arg2 --flag"
                    className="w-full px-3 py-2 text-[13px] bg-white/[0.06] border border-white/[0.1] rounded-md
                               text-white placeholder:text-gray-600 focus:outline-none focus:border-white/[0.2]"
                  />
                </div>
              </div>
            </motion.div>
          )}
        </AnimatePresence>
      </div>
    </div>
  )
}
