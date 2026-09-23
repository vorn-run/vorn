// @vitest-environment jsdom
import { describe, it, expect, beforeEach, vi } from 'vitest'
import '@testing-library/jest-dom/vitest'
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import type {
  Artifact,
  ArtifactComment,
  NodeExecutionState,
  WorkflowNode
} from '../src/shared/types'

const ART: Artifact = {
  id: 'g1',
  kind: 'page',
  title: 'Review the draft',
  sessionId: null,
  projectName: null,
  latestVersion: 1,
  gateRunId: 'run-1',
  gateNodeId: 'review',
  createdAt: '2026-09-23T10:00:00.000Z',
  updatedAt: '2026-09-23T10:00:00.000Z'
}
const draft = (id: string, over: Partial<ArtifactComment> = {}): ArtifactComment => ({
  id,
  artifactId: 'g1',
  version: 1,
  anchor: { kind: 'quote', quote: '79.8% of the time', prefix: 'correctly ', suffix: '…' },
  body: 'Add the baseline right after.',
  state: 'draft',
  createdAt: '2026-09-23T10:00:00.000Z',
  updatedAt: '2026-09-23T10:00:00.000Z',
  ...over
})

let forGate: unknown = null
let comments: ArtifactComment[] = []
const resolveGate = vi.fn(async (_p: unknown) => ({ accepted: true }))

Object.defineProperty(window, 'api', {
  value: {
    getAppVersion: () => '1.0.0',
    getReachableUrls: async () => ({ port: 9000 }),
    artifactForGate: vi.fn(async () => forGate),
    getArtifact: vi.fn(async () => ({ artifact: ART, versions: [], comments, queued: false })),
    onArtifactPublished: () => () => {},
    onArtifactCommentsChanged: () => () => {},
    artifactSelection: vi.fn(async () => null),
    paintArtifactMarks: vi.fn(async () => ({ found: {} })),
    clearArtifactSelection: vi.fn(async () => ({ ok: true })),
    attachBrowser: vi.fn(),
    detachBrowser: vi.fn(),
    resolveWorkflowGate: resolveGate
  },
  writable: true,
  configurable: true
})

const { GateReviewModal } = await import('../src/renderer/components/workflow-runs/GateReviewModal')

const node = {
  id: 'review',
  type: 'approval',
  slug: 'my_review',
  label: 'Review the draft',
  config: { view: '{{steps.page.output}}', feedback: { from: 'write', maxRounds: 3 } },
  position: { x: 0, y: 0 }
} as unknown as WorkflowNode
const write = {
  id: 'write',
  type: 'launchAgent',
  slug: 'write',
  label: 'Write the draft',
  config: {},
  position: { x: 0, y: 0 }
} as unknown as WorkflowNode
const state: NodeExecutionState = {
  nodeId: 'review',
  status: 'waiting',
  round: 1,
  viewToken: 'tok'
}

const open = () =>
  render(
    <GateReviewModal
      runId="run-1"
      workflowName="Weekly notes"
      state={state}
      node={node}
      nodes={[write, node]}
      onClose={vi.fn()}
    />
  )

beforeEach(() => {
  resolveGate.mockClear()
  forGate = { artifact: ART, version: 1, url: 'http://127.0.0.1:9000/artifact/g1/1?t=tok' }
  comments = []
})

describe('the review page at a gate', () => {
  it('shows the page in a guest Vorn reads, with the comments beside it', async () => {
    comments = [draft('c1')]
    const { container } = open()

    await waitFor(() => expect(container.querySelector('webview')).toBeInTheDocument())
    expect(container.querySelector('webview')).toHaveAttribute(
      'src',
      'http://127.0.0.1:9000/artifact/g1/1?t=tok'
    )
    expect(await screen.findByText('Add the baseline right after.')).toBeInTheDocument()
    expect(
      screen.getByText('Sandboxed. Comments are added by Vorn, never by the page.')
    ).toBeInTheDocument()
  })

  it('carries the comments back with Request changes, which then needs no other note', async () => {
    comments = [draft('c1'), draft('c2', { anchor: null, body: 'Lead with the result.' })]
    open()

    fireEvent.click(await screen.findByText('Request changes · 2'))
    fireEvent.click(screen.getByText('Send back'))

    await waitFor(() =>
      expect(resolveGate).toHaveBeenCalledWith({
        runId: 'run-1',
        nodeId: 'review',
        decision: 'changes',
        comments: [
          { quote: '79.8% of the time', comment: 'Add the baseline right after.' },
          { comment: 'Lead with the result.' }
        ]
      })
    )
  })

  it('falls back to the plain page when the gate kept none to comment on', async () => {
    forGate = null
    const { container } = open()

    await waitFor(() => expect(container.querySelector('iframe')).toBeInTheDocument())
    expect(container.querySelector('webview')).not.toBeInTheDocument()
    expect(screen.getByText('Request changes')).toBeInTheDocument()
  })
})
