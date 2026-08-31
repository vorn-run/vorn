import { useCallback, useEffect, useRef } from 'react'
import type { WorkflowEdge, WorkflowNode } from '../../shared/types'

interface Snapshot {
  nodes: WorkflowNode[]
  edges: WorkflowEdge[]
}

const STACK_LIMIT = 100
/** Edits landing this close together read as one gesture — typing a label,
 *  nudging a node — and undo as one step. */
const MERGE_WINDOW_MS = 500

/**
 * Undo/redo over the workflow definition, as snapshots.
 *
 * A definition is kilobytes, so whole-state snapshots buy the same UX as a
 * command system for none of its machinery. The hook watches the definition
 * the editor already owns; applying an undo sets the same state, flagged so
 * the application itself is not recorded as a new edit.
 */
export function useDefinitionHistory(
  nodes: WorkflowNode[],
  edges: WorkflowEdge[],
  setNodes: (nodes: WorkflowNode[]) => void,
  setEdges: (edges: WorkflowEdge[]) => void,
  /** Changes when a different workflow loads; history never crosses it. */
  resetKey: string
): { undo: () => void; redo: () => void } {
  const past = useRef<Snapshot[]>([])
  const future = useRef<Snapshot[]>([])
  const current = useRef<Snapshot>({ nodes, edges })
  const applying = useRef(false)
  const lastPushAt = useRef(0)
  const key = useRef(resetKey)

  useEffect(() => {
    if (key.current !== resetKey) {
      key.current = resetKey
      past.current = []
      future.current = []
      lastPushAt.current = 0
      current.current = { nodes, edges }
      return
    }
    if (nodes === current.current.nodes && edges === current.current.edges) return
    if (applying.current) {
      applying.current = false
      current.current = { nodes, edges }
      return
    }
    const now = Date.now()
    if (now - lastPushAt.current > MERGE_WINDOW_MS || past.current.length === 0) {
      past.current.push(current.current)
      if (past.current.length > STACK_LIMIT) past.current.shift()
    }
    lastPushAt.current = now
    future.current = []
    current.current = { nodes, edges }
  }, [nodes, edges, resetKey])

  const undo = useCallback(() => {
    const snapshot = past.current.pop()
    if (!snapshot) return
    future.current.push(current.current)
    applying.current = true
    lastPushAt.current = 0
    setNodes(snapshot.nodes)
    setEdges(snapshot.edges)
  }, [setNodes, setEdges])

  const redo = useCallback(() => {
    const snapshot = future.current.pop()
    if (!snapshot) return
    past.current.push(current.current)
    applying.current = true
    lastPushAt.current = 0
    setNodes(snapshot.nodes)
    setEdges(snapshot.edges)
  }, [setNodes, setEdges])

  return { undo, redo }
}
