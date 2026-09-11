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
  ArrowRightLeft,
  RotateCcw,
  ChevronRight
} from 'lucide-react'
import type { LucideIcon } from 'lucide-react'
import type {
  ConnectorActionDef,
  ConnectorManifest,
  SdkConnectorIcon,
  SourceConnection,
  TriggerConfig
} from '../../../../shared/types'
import {
  useConnections,
  useConnectorIdFor,
  useConnectionIconFor,
  useInstalledPacks
} from '../../../lib/use-connections'
import { useConnectorCatalog } from '../../../lib/use-connector-catalog'
import { connectionConnectorId } from '../../../lib/connection-icon'
import { HTTP_PROFILE_CONNECTOR } from '../../../../shared/workflow-portability'
import {
  actionKey,
  readOpenGroups,
  readRecentPicks,
  recordRecentPick,
  writeOpenGroups
} from '../../../lib/step-library-memory'
import { ConnectorIcon } from '../../ConnectorIcon'
import { NODE_GLYPH } from '../node-visuals'
import { packStateFor } from '../../../lib/pack-status'
import type { AddableNodeType } from '../WorkflowCanvas'
import type { LibraryPick } from '../../../lib/library-pick'

export type { LibraryPick } from '../../../lib/library-pick'

/** What the anchor that opened the library allows. */
export interface LibraryScope {
  /** A loop body takes only the steps it can repeat. */
  bodyOnly: boolean
  /** Loop and parallel insertion stay off anchors inside a fork branch. */
  insideBranch: boolean
  /** The trigger spot takes only triggers: built-in types and connector events. */
  triggers?: boolean
  /** Swapping a step in place: structural entries (condition, loop, parallel) stay out. */
  replacing?: boolean
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
  { triggerType: 'sessionRestored', label: 'Session restored', icon: RotateCcw },
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

type ConnectorMark = { connectorId: string; icon?: SdkConnectorIcon }

interface Row {
  key: string
  label: string
  pick: LibraryPick
  icon?: LucideIcon
  connection?: SourceConnection
  /** A connector's own mark, for a step that belongs to one nobody has installed. */
  mark?: ConnectorMark
  /** What picking it will also do, or whose it is, said where the eye ends up anyway. */
  detail?: string
  /** Sits under its group's heading, which already shows whose it is. */
  nested?: boolean
  header?: false
}

/**
 * A heading over the rows beneath it.
 *
 * A connection's heading names the connection; the catalog's name the connector,
 * because the rows under them belong to no connection yet — which is the whole
 * difference between what you have and what you could have.
 */
interface GroupHeader {
  key: string
  header: true
  label: string
  count: number
  connection?: SourceConnection
  mark?: ConnectorMark
  detail?: string
  /** Whether its rows show; absent for a heading that never folds. */
  open?: boolean
}

interface Heading {
  key: string
  heading: true
  label: string
}

type Entry = Row | GroupHeader | Heading

interface Group {
  header: GroupHeader
  rows: Row[]
}

const isHeading = (entry: Entry): entry is Heading => 'heading' in entry
const isHeader = (entry: Entry): entry is GroupHeader => 'header' in entry && !!entry.header

function ConnectionMark({ connection }: { connection: SourceConnection }) {
  const connectorId = useConnectorIdFor(connection.id)
  const icon = useConnectionIconFor(connection.id)
  if (!connectorId) return <Zap size={14} className={`${NODE_GLYPH} shrink-0`} strokeWidth={2} />
  return (
    <ConnectorIcon connectorId={connectorId} icon={icon} size={14} className="text-ink shrink-0" />
  )
}

function Mark({ connection, mark }: { connection?: SourceConnection; mark?: ConnectorMark }) {
  if (connection) return <ConnectionMark connection={connection} />
  if (!mark) return null
  return (
    <ConnectorIcon
      connectorId={mark.connectorId}
      icon={mark.icon}
      size={14}
      className="text-ink shrink-0"
    />
  )
}

/** The docked library every + opens: steps, what was picked lately, then each connector folded. */
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
  const [openGroups, setOpenGroups] = useState(readOpenGroups)
  const [recent] = useState(readRecentPicks)
  const inputRef = useRef<HTMLInputElement>(null)
  const connections = useConnections()
  const packs = useInstalledPacks()
  const catalog = useConnectorCatalog()
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

  const entries = useMemo(() => {
    const q = query.trim().toLowerCase()
    const entries: Entry[] = []

    if (scope.triggers) {
      const builtIn: Row[] = TRIGGER_ITEMS.filter(
        (t) => !q || t.label.toLowerCase().includes(q)
      ).map((t) => ({
        key: `trigger:${t.triggerType}`,
        label: t.label,
        icon: t.icon,
        pick: { kind: 'triggerType', triggerType: t.triggerType }
      }))
      if (builtIn.length > 0) {
        entries.push({ key: 'heading:triggers', heading: true, label: 'Triggers' }, ...builtIn)
      }
      for (const conn of connections) {
        const triggers = (manifestsByConnector.get(conn.connectorId)?.triggers ?? []).filter(
          (t) =>
            !q ||
            (t.label || t.type).toLowerCase().includes(q) ||
            conn.name.toLowerCase().includes(q)
        )
        if (triggers.length === 0) continue
        entries.push({
          key: `group:${conn.id}`,
          header: true,
          label: conn.name,
          connection: conn,
          count: triggers.length
        })
        for (const trigger of triggers) {
          entries.push({
            key: `event:${conn.id}:${trigger.type}`,
            label: trigger.label || trigger.type,
            connection: conn,
            nested: true,
            pick: { kind: 'connectorTrigger', connectionId: conn.id, event: trigger.type }
          })
        }
      }
      return entries
    }

    const steps: Row[] = STEP_ITEMS.filter(
      (s) => !scope.bodyOnly || s.type === 'agent' || s.type === 'script'
    )
      .filter((s) => !(scope.insideBranch && s.type === 'loop'))
      .filter((s) => !(scope.replacing && (s.type === 'condition' || s.type === 'loop')))
      .filter((s) => !q || s.label.toLowerCase().includes(q))
      .map((s) => ({
        key: `type:${s.type}`,
        label: s.label,
        icon: s.icon,
        pick: { kind: 'type', type: s.type }
      }))
    if (
      !scope.bodyOnly &&
      !scope.insideBranch &&
      !scope.replacing &&
      (!q || 'parallel branch'.includes(q))
    ) {
      steps.push({
        key: 'parallel',
        label: 'Parallel branch',
        icon: Split,
        pick: { kind: 'parallel' }
      })
    }

    // A saved profile is an HTTP request with the hard part already answered,
    // so it sits directly beneath the request rather than inside a form field.
    if (!scope.bodyOnly && !scope.replacing) {
      const profiles: Row[] = connections
        .filter((c) => connectionConnectorId(c) === HTTP_PROFILE_CONNECTOR)
        .map((profile) => ({
          key: `profile:${profile.id}`,
          label: `Call ${profile.name}`,
          icon: Globe,
          pick: {
            kind: 'httpProfile' as const,
            profileConnectionId: profile.id,
            profileName: profile.name
          }
        }))
        .filter((row) => !q || row.label.toLowerCase().includes(q))
      const afterHttp = steps.findIndex((s) => s.key === 'type:httpRequest')
      steps.splice(afterHttp >= 0 ? afterHttp + 1 : steps.length, 0, ...profiles)
    }
    if (steps.length > 0) {
      if (!q) entries.push({ key: 'heading:steps', heading: true, label: 'Steps' })
      entries.push(...steps)
    }

    const groups: Group[] = []
    if (!scope.bodyOnly) {
      for (const conn of connections) {
        const actions = (actionsByConnection.get(conn.id) ?? []).filter(
          (a) =>
            !q ||
            (a.label || a.type).toLowerCase().includes(q) ||
            conn.name.toLowerCase().includes(q)
        )
        if (actions.length === 0) continue
        groups.push({
          header: {
            key: `group:${conn.id}`,
            header: true,
            label: conn.name,
            connection: conn,
            count: actions.length
          },
          rows: actions.map((action) => {
            const pick = {
              kind: 'connectorAction' as const,
              connectionId: conn.id,
              action: action.type,
              actionLabel: action.label || action.type
            }
            return { key: actionKey(pick), label: pick.actionLabel, connection: conn, pick }
          })
        })
      }
    }

    // Steps from connectors nobody has connected; not while replacing, which rebuilds the node from the pick.
    if (!scope.bodyOnly && !scope.replacing) {
      const connected = new Set(connections.map((conn) => connectionConnectorId(conn)))
      const unvouched: Group[] = []
      for (const entry of catalog.items) {
        if (connected.has(entry.id) || !entry.actions?.length) continue
        const entryMatches =
          !q ||
          entry.name.toLowerCase().includes(q) ||
          (entry.keywords ?? []).some((word) => word.toLowerCase().includes(q))
        const actions = entryMatches
          ? entry.actions
          : entry.actions.filter((action) =>
              (action.label || action.type).toLowerCase().includes(q)
            )
        if (actions.length === 0) continue
        // Say the step someone is about to take: a connector already on disk only wants connecting.
        const state = packStateFor({
          installed: packs.find((pack) => pack.id === entry.id),
          catalogItem: entry
        })
        const mark = { connectorId: entry.id, ...(entry.icon && { icon: entry.icon }) }
        const group: Group = {
          header: {
            key: `catalog-group:${entry.id}`,
            header: true,
            label: entry.name,
            mark,
            count: actions.length,
            detail:
              state.kind === 'installed'
                ? 'add connection'
                : state.kind === 'not-released'
                  ? 'not released yet'
                  : 'install on add'
          },
          rows: actions.map((action) => {
            const pick = {
              kind: 'catalogAction' as const,
              connectorId: entry.id,
              action: action.type,
              actionLabel: action.label || action.type
            }
            return { key: actionKey(pick), label: pick.actionLabel, mark, pick }
          })
        }
        // A connector the factory checked is offered as plainly as an installed one; the rest come after.
        if (entry.verified) groups.push(group)
        else unvouched.push(group)
      }
      groups.push(...unvouched)
    }

    // Searching lists every match in one run, each saying whose it is.
    if (q) {
      for (const { header, rows } of groups) {
        entries.push(...rows.map((row) => ({ ...row, detail: header.label })))
      }
      return entries
    }

    const byKey = new Map(groups.flatMap((g) => g.rows.map((row) => [row.key, { row, g }])))
    const lately = recent.flatMap((pick) => {
      const found = byKey.get(actionKey(pick))
      return found
        ? [{ ...found.row, key: `recent:${found.row.key}`, detail: found.g.header.label }]
        : []
    })
    if (lately.length > 0) {
      entries.push({ key: 'heading:recent', heading: true, label: 'Recent' }, ...lately)
    }
    for (const { header, rows } of groups) {
      const open = openGroups.has(header.key)
      entries.push({ ...header, open })
      if (open) entries.push(...rows.map((row) => ({ ...row, nested: true })))
    }
    return entries
  }, [
    query,
    scope,
    connections,
    packs,
    actionsByConnection,
    manifestsByConnector,
    catalog,
    recent,
    openGroups
  ])

  // A heading that folds can be reached with the keys like a row; one that never folds cannot.
  const reachable = useMemo(
    () =>
      entries.filter(
        (e): e is Row | GroupHeader => !isHeading(e) && (!isHeader(e) || e.open !== undefined)
      ),
    [entries]
  )
  const clamped = Math.min(highlight, Math.max(0, reachable.length - 1))

  const toggleGroup = (key: string, open?: boolean) => {
    const next = new Set(openGroups)
    if (open ?? !next.has(key)) next.add(key)
    else next.delete(key)
    setOpenGroups(next)
    writeOpenGroups(next)
  }

  const choose = (pick: LibraryPick) => {
    if (pick.kind === 'connectorAction' || pick.kind === 'catalogAction') recordRecentPick(pick)
    onPick(pick)
  }

  return (
    <div
      data-step-library
      className="w-[280px] border-l border-white/[0.08] bg-surface-node flex flex-col h-full overflow-hidden titlebar-no-drag"
      onKeyDown={(e) => {
        const current = reachable[clamped]
        if (e.key === 'Escape') {
          e.stopPropagation()
          onClose()
        } else if (e.key === 'ArrowDown') {
          e.preventDefault()
          setHighlight((h) => Math.min(h + 1, Math.max(0, reachable.length - 1)))
        } else if (e.key === 'ArrowUp') {
          e.preventDefault()
          setHighlight((h) => Math.max(h - 1, 0))
        } else if (
          current &&
          isHeader(current) &&
          (e.key === 'ArrowRight' || e.key === 'ArrowLeft')
        ) {
          e.preventDefault()
          toggleGroup(current.key, e.key === 'ArrowRight')
        } else if (e.key === 'Enter' && current) {
          e.preventDefault()
          if (isHeader(current)) toggleGroup(current.key)
          else choose(current.pick)
        }
      }}
    >
      <div className="px-4 py-3 border-b border-white/[0.08]">
        <div className="flex items-center justify-between mb-2.5">
          <span className="text-[13px] font-medium text-white">
            {scope.triggers ? 'Add a trigger' : scope.replacing ? 'Replace step' : 'Add a step'}
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
        {reachable.length === 0 && (
          <div className="px-2.5 py-3 text-[11px] text-gray-500">Nothing matches</div>
        )}
        {(() => {
          const reachIndex = new Map(reachable.map((e, i) => [e.key, i]))
          return entries.map((entry) => {
            if (isHeading(entry)) {
              return (
                <div
                  key={entry.key}
                  className="px-2 pt-3 first:pt-1 pb-1 text-[10px] font-mono uppercase tracking-wider text-gray-600"
                >
                  {entry.label}
                </div>
              )
            }
            const index = reachIndex.get(entry.key) ?? -1
            const lit = index === clamped
            if (isHeader(entry)) {
              const count = (
                <span className="shrink-0 text-[10px] font-mono tabular-nums text-gray-600">
                  {entry.count}
                </span>
              )
              if (entry.open === undefined) {
                return (
                  <div key={entry.key} className="flex items-center gap-2 px-2 pt-3 pb-1">
                    <Mark connection={entry.connection} mark={entry.mark} />
                    <span className="text-[12px] font-semibold text-ink-secondary truncate">
                      {entry.label}
                    </span>
                    <span className="ml-auto">{count}</span>
                  </div>
                )
              }
              return (
                <button
                  key={entry.key}
                  aria-expanded={entry.open}
                  onClick={() => toggleGroup(entry.key)}
                  onMouseEnter={() => setHighlight(index)}
                  className={`w-full flex items-center gap-2 rounded-md px-2 py-1.5 text-left text-[12.5px] transition-colors
                            ${lit ? 'bg-white/[0.06] text-white' : 'text-ink-secondary'}`}
                >
                  <ChevronRight
                    size={12}
                    className={`shrink-0 text-gray-500 transition-transform ${entry.open ? 'rotate-90' : ''}`}
                  />
                  <Mark connection={entry.connection} mark={entry.mark} />
                  <span className="truncate">{entry.label}</span>
                  <span className="ml-auto flex items-center gap-2">
                    {entry.detail && (
                      <span className="shrink-0 text-[10px] text-gray-600">{entry.detail}</span>
                    )}
                    {count}
                  </span>
                </button>
              )
            }
            const Icon = entry.icon
            return (
              <button
                key={entry.key}
                onClick={() => choose(entry.pick)}
                onMouseEnter={() => setHighlight(index)}
                className={`w-full flex items-center gap-2.5 rounded-md text-[12.5px] text-left transition-colors
                          ${entry.nested ? 'pl-8 pr-2 py-1.5' : 'px-2 py-1.5'}
                          ${lit ? 'bg-white/[0.06] text-white' : 'text-gray-300'}`}
              >
                {Icon && <Icon size={14} className={`${NODE_GLYPH} shrink-0`} />}
                {!entry.nested && <Mark connection={entry.connection} mark={entry.mark} />}
                <span className="truncate">{entry.label}</span>
                {entry.detail && (
                  <span className="ml-auto shrink-0 text-[10px] text-gray-600 truncate max-w-[40%]">
                    {entry.detail}
                  </span>
                )}
              </button>
            )
          })
        })()}
      </div>
    </div>
  )
}
