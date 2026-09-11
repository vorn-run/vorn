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
  SourceConnection,
  TriggerConfig
} from '../../../../shared/types'
import {
  connectorLookFor,
  useConnections,
  useInstalledPacks,
  type ConnectorLook
} from '../../../lib/use-connections'
import { useConnectorCatalog } from '../../../lib/use-connector-catalog'
import { connectionConnectorId } from '../../../lib/connection-icon'
import { HTTP_PROFILE_CONNECTOR } from '../../../../shared/workflow-portability'
import {
  readOpenGroups,
  readRecentActions,
  recordRecentAction,
  writeOpenGroups,
  type RecentAction
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

interface Row {
  kind: 'row'
  key: string
  label: string
  pick: LibraryPick
  /** A built-in step's own glyph; connector rows draw their connector's mark instead. */
  icon?: LucideIcon
  look?: ConnectorLook
  /** Whose it is, said when it stands outside its group. */
  owner?: string
  /** How it is offered again under Recent once picked. */
  recent?: RecentAction
  nested?: boolean
}

/** A connection's or a catalog connector's heading, which folds its rows away. */
interface GroupHeader {
  kind: 'group'
  key: string
  label: string
  count: number
  look?: ConnectorLook
  /** What adding from it will also do. */
  detail?: string
  open: boolean
}

interface Heading {
  kind: 'heading'
  key: string
  label: string
}

type Entry = Row | GroupHeader | Heading

interface Group {
  key: string
  label: string
  look?: ConnectorLook
  detail?: string
  rows: Row[]
}

function Mark({ look }: { look?: ConnectorLook }) {
  if (!look?.connectorId) {
    return <Zap size={14} className={`${NODE_GLYPH} shrink-0`} strokeWidth={2} />
  }
  return (
    <ConnectorIcon
      connectorId={look.connectorId}
      icon={look.icon}
      packaged={look.packaged}
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
  const { bodyOnly, insideBranch, triggers, replacing } = scope
  const [query, setQuery] = useState('')
  const [highlight, setHighlight] = useState(0)
  const [openGroups, setOpenGroups] = useState(readOpenGroups)
  const [recent] = useState(readRecentActions)
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
    if (!triggers) return
    window.api.listConnectors().then((connectors) => {
      setManifestsByConnector(new Map(connectors.map((c) => [c.id, c.manifest])))
    })
  }, [triggers])

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

  const q = query.trim().toLowerCase()

  // What matches, in the order it is offered: the loose steps, then one group per connector.
  const { flat, groups } = useMemo(() => {
    const matches = (...texts: string[]) => !q || texts.some((t) => t.toLowerCase().includes(q))
    const flat: Row[] = []
    const groups: Group[] = []
    const connectionGroup = (
      conn: SourceConnection,
      rows: Omit<Row, 'kind' | 'look' | 'owner'>[]
    ) => {
      const look = connectorLookFor(connections, conn.id)
      if (rows.length === 0) return
      groups.push({
        key: `group:${conn.id}`,
        label: conn.name,
        look,
        rows: rows.map((row) => ({ ...row, kind: 'row', look, owner: conn.name }))
      })
    }

    if (triggers) {
      for (const t of TRIGGER_ITEMS) {
        if (!matches(t.label)) continue
        flat.push({
          kind: 'row',
          key: `trigger:${t.triggerType}`,
          label: t.label,
          icon: t.icon,
          pick: { kind: 'triggerType', triggerType: t.triggerType }
        })
      }
      for (const conn of connections) {
        connectionGroup(
          conn,
          (manifestsByConnector.get(conn.connectorId)?.triggers ?? [])
            .filter((t) => matches(t.label || t.type, conn.name))
            .map((t) => ({
              key: `event:${conn.id}:${t.type}`,
              label: t.label || t.type,
              pick: { kind: 'connectorTrigger', connectionId: conn.id, event: t.type }
            }))
        )
      }
      return { flat, groups }
    }

    for (const s of STEP_ITEMS) {
      if (bodyOnly && s.type !== 'agent' && s.type !== 'script') continue
      if (insideBranch && s.type === 'loop') continue
      if (replacing && (s.type === 'condition' || s.type === 'loop')) continue
      if (!matches(s.label)) continue
      flat.push({
        kind: 'row',
        key: `type:${s.type}`,
        label: s.label,
        icon: s.icon,
        pick: { kind: 'type', type: s.type }
      })
    }
    if (!bodyOnly && !insideBranch && !replacing && matches('Parallel branch')) {
      flat.push({
        kind: 'row',
        key: 'parallel',
        label: 'Parallel branch',
        icon: Split,
        pick: { kind: 'parallel' }
      })
    }

    // A saved profile is an HTTP request with the hard part already answered,
    // so it sits directly beneath the request rather than inside a form field.
    if (!bodyOnly && !replacing) {
      const profiles: Row[] = connections
        .filter((c) => connectionConnectorId(c) === HTTP_PROFILE_CONNECTOR)
        .filter((profile) => matches(`Call ${profile.name}`))
        .map((profile) => ({
          kind: 'row',
          key: `profile:${profile.id}`,
          label: `Call ${profile.name}`,
          icon: Globe,
          pick: {
            kind: 'httpProfile',
            profileConnectionId: profile.id,
            profileName: profile.name
          }
        }))
      const afterHttp = flat.findIndex((s) => s.key === 'type:httpRequest')
      flat.splice(afterHttp >= 0 ? afterHttp + 1 : flat.length, 0, ...profiles)
    }

    if (!bodyOnly) {
      for (const conn of connections) {
        const connectorId = connectionConnectorId(conn)
        connectionGroup(
          conn,
          (actionsByConnection.get(conn.id) ?? [])
            .filter((a) => matches(a.label || a.type, conn.name))
            .map((a) => ({
              key: `action:${conn.id}:${a.type}`,
              label: a.label || a.type,
              ...(connectorId && {
                recent: { connectorId, action: a.type, connectionId: conn.id }
              }),
              pick: {
                kind: 'connectorAction',
                connectionId: conn.id,
                action: a.type,
                actionLabel: a.label || a.type
              }
            }))
        )
      }
    }

    // Steps from connectors nobody has connected; not while replacing, which rebuilds the node from the pick.
    if (!bodyOnly && !replacing) {
      const connected = new Set(connections.map((conn) => connectionConnectorId(conn)))
      const unvouched: Group[] = []
      for (const entry of catalog.items) {
        if (connected.has(entry.id) || !entry.actions?.length) continue
        const actions = matches(entry.name, ...(entry.keywords ?? []))
          ? entry.actions
          : entry.actions.filter((action) => matches(action.label || action.type))
        if (actions.length === 0) continue
        // Say the step someone is about to take: a connector already on disk only wants connecting.
        const state = packStateFor({
          installed: packs.find((pack) => pack.id === entry.id),
          catalogItem: entry
        })
        const look = { connectorId: entry.id, icon: entry.icon, packaged: false }
        const group: Group = {
          key: `catalog-group:${entry.id}`,
          label: entry.name,
          look,
          detail:
            state.kind === 'installed'
              ? 'add connection'
              : state.kind === 'not-released'
                ? 'not released yet'
                : 'install on add',
          rows: actions.map((action) => ({
            kind: 'row',
            key: `catalog:${entry.id}:${action.type}`,
            label: action.label || action.type,
            look,
            owner: entry.name,
            recent: { connectorId: entry.id, action: action.type },
            pick: {
              kind: 'catalogAction',
              connectorId: entry.id,
              action: action.type,
              actionLabel: action.label || action.type
            }
          }))
        }
        // A connector the factory checked is offered as plainly as an installed one; the rest come after.
        if (entry.verified) groups.push(group)
        else unvouched.push(group)
      }
      groups.push(...unvouched)
    }

    return { flat, groups }
  }, [
    q,
    bodyOnly,
    insideBranch,
    triggers,
    replacing,
    connections,
    packs,
    actionsByConnection,
    manifestsByConnector,
    catalog
  ])

  // Searching lists every match in one run; otherwise groups fold, under what was picked lately.
  const entries = useMemo(() => {
    if (q) return [...flat, ...groups.flatMap((g) => g.rows)]
    const entries: Entry[] = []
    if (flat.length > 0) {
      entries.push({ kind: 'heading', key: 'heading:top', label: triggers ? 'Triggers' : 'Steps' })
      entries.push(...flat)
    }
    const offered = groups.flatMap((g) => g.rows)
    const lately = recent.flatMap((r) => {
      const same = offered.filter(
        (row) => row.recent?.connectorId === r.connectorId && row.recent.action === r.action
      )
      const row = same.find((s) => s.recent?.connectionId === r.connectionId) ?? same[0]
      return row ? [{ ...row, key: `recent:${row.key}` }] : []
    })
    if (lately.length > 0) {
      entries.push({ kind: 'heading', key: 'heading:recent', label: 'Recent' }, ...lately)
    }
    for (const g of groups) {
      const open = openGroups.has(g.key)
      entries.push({
        kind: 'group',
        key: g.key,
        label: g.label,
        look: g.look,
        detail: g.detail,
        count: g.rows.length,
        open
      })
      if (open) entries.push(...g.rows.map((row) => ({ ...row, nested: true })))
    }
    return entries
  }, [q, flat, groups, triggers, recent, openGroups])

  const { reachable, reachIndex } = useMemo(() => {
    const reachable = entries.filter((e): e is Row | GroupHeader => e.kind !== 'heading')
    return { reachable, reachIndex: new Map(reachable.map((e, i) => [e.key, i])) }
  }, [entries])
  const clamped = Math.min(highlight, Math.max(0, reachable.length - 1))

  const toggleGroup = (key: string, open = !openGroups.has(key)) => {
    if (open === openGroups.has(key)) return
    const next = new Set(openGroups)
    if (open) next.add(key)
    else next.delete(key)
    setOpenGroups(next)
    writeOpenGroups(next)
  }

  const choose = (row: Row) => {
    if (row.recent) recordRecentAction(row.recent)
    onPick(row.pick)
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
        } else if (current?.kind === 'group' && (e.key === 'ArrowRight' || e.key === 'ArrowLeft')) {
          e.preventDefault()
          toggleGroup(current.key, e.key === 'ArrowRight')
        } else if (e.key === 'Enter' && current) {
          e.preventDefault()
          if (current.kind === 'group') toggleGroup(current.key)
          else choose(current)
        }
      }}
    >
      <div className="px-4 py-3 border-b border-white/[0.08]">
        <div className="flex items-center justify-between mb-2.5">
          <span className="text-[13px] font-medium text-white">
            {triggers ? 'Add a trigger' : replacing ? 'Replace step' : 'Add a step'}
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
            placeholder={triggers ? 'Search triggers' : 'Search steps and actions'}
            className="w-full bg-transparent text-[12px] text-white placeholder:text-gray-600 outline-none"
          />
        </div>
      </div>

      <div className="flex-1 overflow-y-auto p-2">
        {reachable.length === 0 && (
          <div className="px-2.5 py-3 text-[11px] text-gray-500">Nothing matches</div>
        )}
        {entries.map((entry) => {
          if (entry.kind === 'heading') {
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
          const tone = index === clamped ? 'bg-white/[0.06] text-white' : ''
          if (entry.kind === 'group') {
            return (
              <button
                key={entry.key}
                aria-expanded={entry.open}
                onClick={() => toggleGroup(entry.key)}
                onMouseEnter={() => setHighlight(index)}
                className={`w-full flex items-center gap-2 rounded-md px-2 py-1.5 text-left text-[12.5px] transition-colors
                          ${tone || 'text-ink-secondary'}`}
              >
                <ChevronRight
                  size={12}
                  className={`shrink-0 text-gray-500 transition-transform ${entry.open ? 'rotate-90' : ''}`}
                />
                <Mark look={entry.look} />
                <span className="truncate">{entry.label}</span>
                <span className="ml-auto flex items-center gap-2 shrink-0 text-[10px] text-gray-600">
                  {entry.detail}
                  <span className="font-mono tabular-nums">{entry.count}</span>
                </span>
              </button>
            )
          }
          const Icon = entry.icon
          return (
            <button
              key={entry.key}
              onClick={() => choose(entry)}
              onMouseEnter={() => setHighlight(index)}
              className={`w-full flex items-center gap-2.5 rounded-md text-[12.5px] text-left transition-colors
                        ${entry.nested ? 'pl-8 pr-2 py-1.5' : 'px-2 py-1.5'} ${tone || 'text-gray-300'}`}
            >
              {Icon ? (
                <Icon size={14} className={`${NODE_GLYPH} shrink-0`} />
              ) : (
                !entry.nested && <Mark look={entry.look} />
              )}
              <span className="truncate">{entry.label}</span>
              {!entry.nested && entry.owner && (
                <span className="ml-auto shrink-0 max-w-[40%] truncate text-[10px] text-gray-600">
                  {entry.owner}
                </span>
              )}
            </button>
          )
        })}
      </div>
    </div>
  )
}
