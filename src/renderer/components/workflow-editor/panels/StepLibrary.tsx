import { useEffect, useMemo, useRef, useState } from 'react'
import {
  Play,
  Terminal,
  GitFork,
  Hand,
  Repeat,
  Split,
  Search,
  X,
  Zap,
  Globe,
  Clock,
  Calendar,
  ListPlus,
  ArrowRightLeft
} from 'lucide-react'
import type { LucideIcon } from 'lucide-react'
import type {
  ConnectorActionDef,
  ConnectorManifest,
  SourceConnection,
  TriggerConfig
} from '../../../../shared/types'
import {
  useConnections,
  useConnectorIdFor,
  useConnectionIconFor
} from '../../../lib/use-connections'
import { ConnectorIcon } from '../../ConnectorIcon'
import { NODE_GLYPH } from '../node-visuals'
import type { AddableNodeType } from '../WorkflowCanvas'

/** A step type, a parallel branch, a connector action, or a trigger. */
export type LibraryPick =
  | { kind: 'type'; type: AddableNodeType }
  | { kind: 'parallel' }
  | { kind: 'connectorAction'; connectionId: string; action: string }
  | { kind: 'triggerType'; triggerType: TriggerConfig['triggerType'] }
  | { kind: 'connectorTrigger'; connectionId: string; event: string }

/** What the anchor that opened the library allows. */
export interface LibraryScope {
  /** A loop body takes only the steps it can repeat. */
  bodyOnly: boolean
  /** Loop and parallel insertion stay off anchors inside a fork branch. */
  insideBranch: boolean
  /** The trigger spot takes only triggers: built-in types and connector events. */
  triggers?: boolean
}

const TRIGGER_ITEMS: {
  triggerType: TriggerConfig['triggerType']
  label: string
  icon: LucideIcon
}[] = [
  { triggerType: 'manual', label: 'Manual', icon: Zap },
  { triggerType: 'recurring', label: 'Recurring schedule', icon: Clock },
  { triggerType: 'once', label: 'Schedule once', icon: Calendar },
  { triggerType: 'taskCreated', label: 'Task created', icon: ListPlus },
  { triggerType: 'taskStatusChanged', label: 'Task moved', icon: ArrowRightLeft },
  { triggerType: 'webhook', label: 'Webhook', icon: Globe }
]

const STEP_ITEMS: { type: AddableNodeType; label: string; icon: LucideIcon }[] = [
  { type: 'agent', label: 'Agent', icon: Play },
  { type: 'script', label: 'Script', icon: Terminal },
  { type: 'httpRequest', label: 'HTTP request', icon: Globe },
  { type: 'condition', label: 'Condition', icon: GitFork },
  { type: 'approval', label: 'Approval gate', icon: Hand },
  { type: 'loop', label: 'Loop', icon: Repeat }
]

interface Row {
  key: string
  label: string
  pick: LibraryPick
  icon?: LucideIcon
  connection?: SourceConnection
  header?: false
}

interface GroupHeader {
  key: string
  header: true
  connection: SourceConnection
  count: number
}

function ConnectionMark({ connection }: { connection: SourceConnection }) {
  const connectorId = useConnectorIdFor(connection.id)
  const icon = useConnectionIconFor(connection.id)
  if (!connectorId) return <Zap size={14} className={`${NODE_GLYPH} shrink-0`} strokeWidth={2} />
  return (
    <ConnectorIcon connectorId={connectorId} icon={icon} size={14} className="text-ink shrink-0" />
  )
}

/** The docked library every + opens: steps first, then each connection's actions. */
export function StepLibrary({
  scope,
  onPick,
  onClose
}: {
  scope: LibraryScope
  onPick: (pick: LibraryPick) => void
  onClose: () => void
}) {
  const [query, setQuery] = useState('')
  const [highlight, setHighlight] = useState(0)
  const inputRef = useRef<HTMLInputElement>(null)
  const connections = useConnections()
  const [actionsByConnection, setActionsByConnection] = useState<Map<string, ConnectorActionDef[]>>(
    () => new Map()
  )
  const [manifestsByConnector, setManifestsByConnector] = useState<Map<string, ConnectorManifest>>(
    () => new Map()
  )

  useEffect(() => {
    if (!scope.triggers) return
    window.api.listConnectors().then((connectors) => {
      setManifestsByConnector(new Map(connectors.map((c) => [c.id, c.manifest])))
    })
  }, [scope.triggers])

  useEffect(() => {
    inputRef.current?.focus()
  }, [])

  useEffect(() => {
    let cancelled = false
    Promise.all(
      connections.map(async (conn): Promise<[string, ConnectorActionDef[]]> => {
        try {
          return [conn.id, await window.api.listConnectionActions(conn.id)]
        } catch {
          return [conn.id, []]
        }
      })
    ).then((entries) => {
      if (!cancelled) setActionsByConnection(new Map(entries))
    })
    return () => {
      cancelled = true
    }
  }, [connections])

  const { rows, pickable } = useMemo(() => {
    const q = query.trim().toLowerCase()
    const rows: (Row | GroupHeader)[] = []

    if (scope.triggers) {
      rows.push(
        ...TRIGGER_ITEMS.filter((t) => !q || t.label.toLowerCase().includes(q)).map((t) => ({
          key: `trigger:${t.triggerType}`,
          label: t.label,
          icon: t.icon,
          pick: { kind: 'triggerType', triggerType: t.triggerType } as LibraryPick
        }))
      )
      for (const conn of connections) {
        const triggers = (manifestsByConnector.get(conn.connectorId)?.triggers ?? []).filter(
          (t) =>
            !q ||
            (t.label || t.type).toLowerCase().includes(q) ||
            conn.name.toLowerCase().includes(q)
        )
        if (triggers.length === 0) continue
        rows.push({
          key: `group:${conn.id}`,
          header: true,
          connection: conn,
          count: triggers.length
        })
        for (const trigger of triggers) {
          rows.push({
            key: `event:${conn.id}:${trigger.type}`,
            label: trigger.label || trigger.type,
            connection: conn,
            pick: { kind: 'connectorTrigger', connectionId: conn.id, event: trigger.type }
          })
        }
      }
      return { rows, pickable: rows.filter((r): r is Row => !('header' in r && r.header)) }
    }

    const steps: Row[] = STEP_ITEMS.filter(
      (s) => !scope.bodyOnly || s.type === 'agent' || s.type === 'script'
    )
      .filter((s) => !(scope.insideBranch && s.type === 'loop'))
      .filter((s) => !q || s.label.toLowerCase().includes(q))
      .map((s) => ({
        key: `type:${s.type}`,
        label: s.label,
        icon: s.icon,
        pick: { kind: 'type', type: s.type }
      }))
    if (!scope.bodyOnly && !scope.insideBranch && (!q || 'parallel branch'.includes(q))) {
      steps.push({
        key: 'parallel',
        label: 'Parallel branch',
        icon: Split,
        pick: { kind: 'parallel' }
      })
    }
    if (steps.length > 0) rows.push(...steps)

    if (!scope.bodyOnly) {
      for (const conn of connections) {
        const actions = (actionsByConnection.get(conn.id) ?? []).filter(
          (a) =>
            !q ||
            (a.label || a.type).toLowerCase().includes(q) ||
            conn.name.toLowerCase().includes(q)
        )
        if (actions.length === 0) continue
        rows.push({
          key: `group:${conn.id}`,
          header: true,
          connection: conn,
          count: actions.length
        })
        for (const action of actions) {
          rows.push({
            key: `action:${conn.id}:${action.type}`,
            label: action.label || action.type,
            connection: conn,
            pick: { kind: 'connectorAction', connectionId: conn.id, action: action.type }
          })
        }
      }
    }

    return { rows, pickable: rows.filter((r): r is Row => !('header' in r && r.header)) }
  }, [query, scope, connections, actionsByConnection, manifestsByConnector])

  const clamped = Math.min(highlight, Math.max(0, pickable.length - 1))

  return (
    <div
      data-step-library
      className="w-[280px] border-l border-white/[0.08] bg-surface-node flex flex-col h-full overflow-hidden titlebar-no-drag"
      onKeyDown={(e) => {
        if (e.key === 'Escape') {
          e.stopPropagation()
          onClose()
        } else if (e.key === 'ArrowDown') {
          e.preventDefault()
          setHighlight((h) => Math.min(h + 1, Math.max(0, pickable.length - 1)))
        } else if (e.key === 'ArrowUp') {
          e.preventDefault()
          setHighlight((h) => Math.max(h - 1, 0))
        } else if (e.key === 'Enter' && pickable[clamped]) {
          e.preventDefault()
          onPick(pickable[clamped].pick)
        }
      }}
    >
      <div className="px-4 py-3 border-b border-white/[0.08]">
        <div className="flex items-center justify-between mb-2.5">
          <span className="text-[13px] font-medium text-white">
            {scope.triggers ? 'Add a trigger' : 'Add a step'}
          </span>
          <button
            aria-label="Close"
            onClick={onClose}
            className="p-1 rounded-md text-gray-500 hover:text-white hover:bg-white/[0.06] transition-colors"
          >
            <X size={14} />
          </button>
        </div>
        <div className="flex items-center gap-1.5 border border-white/[0.08] rounded-md px-2 py-1.5">
          <Search size={12} className="shrink-0 text-gray-500" />
          <input
            ref={inputRef}
            value={query}
            onChange={(e) => {
              setQuery(e.target.value)
              setHighlight(0)
            }}
            placeholder={scope.triggers ? 'Search triggers' : 'Search steps and actions'}
            className="w-full bg-transparent text-[12px] text-white placeholder:text-gray-600 outline-none"
          />
        </div>
      </div>

      <div className="flex-1 overflow-y-auto p-2">
        {pickable.length === 0 && (
          <div className="px-2.5 py-3 text-[11px] text-gray-500">Nothing matches</div>
        )}
        {pickable.length > 0 && rows[0] && !('header' in rows[0] && rows[0].header) && (
          <div className="px-2 pt-1 pb-1 text-[10px] font-mono uppercase tracking-wider text-gray-600">
            {scope.triggers ? 'Triggers' : 'Steps'}
          </div>
        )}
        {(() => {
          const pickIndex = new Map(pickable.map((r, i) => [r.key, i]))
          return rows.map((row) => {
            if ('header' in row && row.header) {
              return (
                <div key={row.key} className="flex items-center gap-2 px-2 pt-3 pb-1">
                  <ConnectionMark connection={row.connection} />
                  <span className="text-[12px] font-semibold text-ink-secondary truncate">
                    {row.connection.name}
                  </span>
                  <span className="ml-auto text-[10px] font-mono text-gray-600">{row.count}</span>
                </div>
              )
            }
            const index = pickIndex.get(row.key) ?? -1
            const Icon = row.icon
            return (
              <button
                key={row.key}
                onClick={() => onPick(row.pick)}
                onMouseEnter={() => setHighlight(index)}
                className={`w-full flex items-center gap-2.5 rounded-md text-[12.5px] text-left transition-colors
                          ${row.connection ? 'pl-8 pr-2 py-1.5' : 'px-2 py-1.5'}
                          ${index === clamped ? 'bg-white/[0.06] text-white' : 'text-gray-300'}`}
              >
                {Icon && <Icon size={14} className={`${NODE_GLYPH} shrink-0`} />}
                <span className="truncate">{row.label}</span>
              </button>
            )
          })
        })()}
      </div>
    </div>
  )
}
