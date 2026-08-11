import { useId, useState } from 'react'
import { ChevronDown, ChevronRight, RefreshCw, Wrench, AlertCircle } from 'lucide-react'
import { Tooltip } from '../Tooltip'
import { schemaProperties, schemaTypeHint } from '../../../shared/json-schema-utils'
import type { SourceConnection } from '../../../shared/types'

interface McpTool {
  name: string
  description?: string
  inputSchema?: Record<string, unknown>
}

/**
 * The tools a raw MCP connection discovered, and a way to call one.
 *
 * Only reachable from an MCP connection row, so it lives beside it rather than
 * in the settings page that used to hold everything.
 */
export function McpToolsPanel({
  connection,
  onRefresh
}: {
  connection: SourceConnection
  onRefresh: () => void | Promise<void>
}) {
  const [expanded, setExpanded] = useState(false)
  const [refreshing, setRefreshing] = useState(false)
  const [refreshError, setRefreshError] = useState<string | null>(null)
  const [selectedTool, setSelectedTool] = useState<McpTool | null>(null)

  const tools = Array.isArray(connection.filters.discoveredTools)
    ? (connection.filters.discoveredTools as McpTool[])
    : []

  const handleRefresh = async () => {
    setRefreshing(true)
    setRefreshError(null)
    try {
      const result = await window.api.refreshMcpTools(connection.id)
      if (!result.ok) setRefreshError(result.error ?? 'Refresh failed')
      // Pull fresh connections into the parent so `filters.discoveredTools`
      // on this row reflects whatever the server just wrote.
      await onRefresh()
    } catch (err) {
      setRefreshError(err instanceof Error ? err.message : String(err))
    } finally {
      setRefreshing(false)
    }
  }

  return (
    <div className="mt-2 border-t border-white/[0.06] pt-2">
      <div className="flex items-center justify-between">
        <button
          onClick={() => setExpanded(!expanded)}
          className="flex items-center gap-1 text-[11px] text-gray-400 hover:text-white transition-colors"
        >
          {expanded ? <ChevronDown size={11} /> : <ChevronRight size={11} />}
          <Wrench size={11} />
          <span>
            {tools.length > 0
              ? `${tools.length} tool${tools.length === 1 ? '' : 's'}`
              : 'No tools discovered yet'}
          </span>
        </button>
        <Tooltip label="Re-run tools/list on the MCP server">
          <button
            onClick={handleRefresh}
            disabled={refreshing}
            className="p-1 text-gray-500 hover:text-gray-200 rounded-sm transition-colors disabled:opacity-50"
          >
            <RefreshCw size={11} className={refreshing ? 'animate-spin' : ''} />
          </button>
        </Tooltip>
      </div>

      {refreshError && (
        <div className="mt-1 text-[11px] text-red-400 flex items-start gap-1">
          <AlertCircle size={10} className="mt-0.5 shrink-0" />
          <span>{refreshError}</span>
        </div>
      )}

      {expanded && (
        <div className="mt-2 space-y-1">
          {tools.map((tool) => (
            <div
              key={tool.name}
              className="flex items-start justify-between gap-2 px-2 py-1 bg-white/[0.02] rounded-sm"
            >
              <div className="min-w-0">
                <div className="text-[12px] text-gray-200 font-mono truncate">{tool.name}</div>
                {tool.description && (
                  <div className="text-[10px] text-gray-500 leading-snug">{tool.description}</div>
                )}
              </div>
              <button
                onClick={() => setSelectedTool(tool)}
                className="text-[10px] text-gray-400 hover:text-white px-2 py-0.5 border border-white/[0.1] rounded-sm hover:bg-white/[0.06] transition-colors shrink-0"
              >
                Invoke
              </button>
            </div>
          ))}
          {tools.length === 0 && !refreshError && (
            <div className="text-[11px] text-gray-500 italic">
              Click refresh to run tools/list, or wait for initial discovery to complete.
            </div>
          )}
        </div>
      )}

      {selectedTool && (
        // Key by tool name so switching to a different tool remounts the
        // dialog with a fresh args stub + cleared result, instead of leaving
        // the previous tool's state behind.
        <InvokeToolDialog
          key={selectedTool.name}
          connectionId={connection.id}
          tool={selectedTool}
          onClose={() => setSelectedTool(null)}
        />
      )}
    </div>
  )
}

/**
 * Build a skeleton args object from an MCP tool's JSON Schema so the invoke
 * textarea isn't just `{}`. Covers the common cases (string/number/boolean/
 * array/object); unknowns fall back to null.
 */
function buildArgsStub(schema: Record<string, unknown> | undefined): string {
  const props = schemaProperties(schema)
  if (Object.keys(props).length === 0) return '{}'
  const stub: Record<string, unknown> = {}
  for (const [key, raw] of Object.entries(props)) {
    const def = (raw as { default?: unknown }).default
    if (def !== undefined) {
      stub[key] = def
      continue
    }
    switch (schemaTypeHint(raw)) {
      case 'string':
        stub[key] = ''
        break
      case 'number':
      case 'integer':
        stub[key] = 0
        break
      case 'boolean':
        stub[key] = false
        break
      case 'array':
        stub[key] = []
        break
      case 'object':
        stub[key] = {}
        break
      default:
        stub[key] = null
    }
  }
  return JSON.stringify(stub, null, 2)
}

function InvokeToolDialog({
  connectionId,
  tool,
  onClose
}: {
  connectionId: string
  tool: McpTool
  onClose: () => void
}) {
  const [argsText, setArgsText] = useState(() => buildArgsStub(tool.inputSchema))
  // Tied to the textarea below, so the label is not just text sitting near a
  // box a screen reader announces as unnamed.
  const argsId = useId()
  const [running, setRunning] = useState(false)
  const [result, setResult] = useState<{
    success: boolean
    output?: Record<string, unknown>
    error?: string
  } | null>(null)

  const handleRun = async () => {
    let args: Record<string, unknown>
    try {
      args = argsText.trim() === '' ? {} : JSON.parse(argsText)
    } catch (err) {
      setResult({
        success: false,
        error: `Invalid JSON: ${err instanceof Error ? err.message : String(err)}`
      })
      return
    }
    setRunning(true)
    try {
      const res = await window.api.executeConnectorAction({
        connectionId,
        action: tool.name,
        args
      })
      setResult(res)
    } finally {
      setRunning(false)
    }
  }

  return (
    <div className="mt-2 px-2 py-2 bg-black/30 border border-white/[0.08] rounded-sm">
      <div className="flex items-center justify-between mb-2">
        <span className="text-[11px] font-mono text-gray-200">{tool.name}</span>
        <button
          onClick={onClose}
          className="text-[10px] text-gray-500 hover:text-gray-300 transition-colors"
        >
          Close
        </button>
      </div>
      {tool.inputSchema && (
        <details className="mb-2">
          <summary className="text-[10px] text-gray-500 cursor-pointer">inputSchema</summary>
          <pre className="mt-1 text-[10px] text-gray-400 whitespace-pre-wrap break-words max-h-32 overflow-auto">
            {JSON.stringify(tool.inputSchema, null, 2)}
          </pre>
        </details>
      )}
      <label htmlFor={argsId} className="block text-[10px] text-gray-500 mb-1">
        Arguments (JSON)
      </label>
      <textarea
        id={argsId}
        value={argsText}
        onChange={(e) => setArgsText(e.target.value)}
        rows={4}
        className="w-full px-2 py-1 bg-white/[0.05] border border-white/[0.1] rounded-sm text-[11px] font-mono text-gray-200 focus:border-white/[0.2] outline-none"
      />
      <div className="mt-2 flex gap-2">
        <button
          onClick={handleRun}
          disabled={running}
          className="px-3 py-1 text-[11px] bg-white/[0.1] hover:bg-white/[0.15] text-white rounded-sm transition-colors disabled:opacity-50"
        >
          {running ? 'Running…' : 'Run'}
        </button>
      </div>
      {result && (
        <div className="mt-2">
          <div
            className={
              result.success
                ? 'text-[11px] text-green-400'
                : 'text-[11px] text-red-400 flex items-start gap-1'
            }
          >
            {result.success ? (
              <span>Success</span>
            ) : (
              <>
                <AlertCircle size={10} className="mt-0.5 shrink-0" />
                <span>{result.error}</span>
              </>
            )}
          </div>
          {result.output && (
            <pre className="mt-1 text-[10px] text-gray-400 whitespace-pre-wrap break-words max-h-48 overflow-auto bg-black/30 p-2 rounded-sm">
              {JSON.stringify(result.output, null, 2)}
            </pre>
          )}
        </div>
      )}
    </div>
  )
}
