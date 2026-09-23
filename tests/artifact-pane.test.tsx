// @vitest-environment jsdom
import { describe, it, expect, beforeEach, vi } from 'vitest'
import '@testing-library/jest-dom/vitest'
import { render, screen, fireEvent, act, waitFor } from '@testing-library/react'
import type {
  Artifact,
  ArtifactComment,
  ArtifactVersion,
  BrowserTabArtifact
} from '../src/shared/types'

vi.mock('../src/renderer/components/rich-editor/RichMarkdownEditor', () => ({
  RichMarkdownEditor: ({ value, onChange }: { value: string; onChange: (v: string) => void }) => (
    <textarea aria-label="Doc words" value={value} onChange={(e) => onChange(e.target.value)} />
  )
}))

Object.defineProperty(window, 'matchMedia', {
  value: () => ({ matches: false, addEventListener: vi.fn(), removeEventListener: vi.fn() }),
  writable: true,
  configurable: true
})

const ART: Artifact = {
  id: 'a1',
  kind: 'page',
  title: 'Figures for the triage article',
  sessionId: 't1',
  projectName: 'repo',
  latestVersion: 4,
  createdAt: '2026-09-23T10:00:00.000Z',
  updatedAt: '2026-09-23T10:00:00.000Z'
}
const version = (n: number, over: Partial<ArtifactVersion> = {}): ArtifactVersion => ({
  artifactId: 'a1',
  version: n,
  author: 'agent',
  createdAt: new Date().toISOString(),
  ...over
})
const comment = (id: string, over: Partial<ArtifactComment> = {}): ArtifactComment => ({
  id,
  artifactId: 'a1',
  version: 3,
  anchor: { kind: 'quote', quote: 'two API models', prefix: 'and ', suffix: ', a frontier' },
  body: 'Name them.',
  state: 'draft',
  createdAt: '2026-09-23T10:00:00.000Z',
  updatedAt: '2026-09-23T10:00:00.000Z',
  ...over
})

let artifactState: {
  artifact: Artifact
  versions: ArtifactVersion[]
  comments: ArtifactComment[]
  queued: boolean
} | null = null
const mockVersionUrl = vi.fn(async (id: string, n?: number) => ({
  path: `/artifact/${id}/${n}?t=tok`,
  url: `http://127.0.0.1:9000/artifact/${id}/${n}?t=tok`
}))
const mockSend = vi.fn(async () => ({ state: 'delivered' as const, count: 2 }))
const mockSelection = vi.fn(async () => null as unknown)
const mockPaint = vi.fn(async () => ({ found: {} as Record<string, boolean> }))
const mockSave = vi.fn(async (p: unknown) => p)
const mockReadSource = vi.fn(async () => ({
  version: version(4),
  body: '# Triage\n\nIt always bothered me.\n'
}))
const mockSaveUser = vi.fn(async (_p: unknown) => ({
  version: version(5, { author: 'user' }),
  sent: { state: 'delivered' as const, count: 1 }
}))

Object.defineProperty(window, 'api', {
  value: {
    attachBrowser: vi.fn(),
    detachBrowser: vi.fn(),
    syncBrowserTabs: vi.fn(),
    watchBrowserFile: vi.fn(),
    onBrowserFileChanged: () => () => {},
    readBrowserManifest: async () => ({ manifest: null }),
    setBrowserTweak: async () => ({ ok: true }),
    cancelBrowserPick: vi.fn(),
    startBrowserPick: vi.fn(),
    annotateBrowser: vi.fn(),
    writeTerminal: vi.fn(),
    getArtifact: async () => artifactState,
    artifactVersionUrl: (id: string, n?: number) => mockVersionUrl(id, n),
    onArtifactPublished: () => () => {},
    onArtifactCommentsChanged: () => () => {},
    saveArtifactComment: (p: unknown) => mockSave(p),
    updateArtifactComment: vi.fn(async () => null),
    deleteArtifactComment: vi.fn(async () => ({ deleted: true })),
    sendArtifactComments: () => mockSend(),
    artifactSelection: () => mockSelection(),
    paintArtifactMarks: () => mockPaint(),
    revealArtifactMark: vi.fn(async () => ({ ok: true })),
    clearArtifactSelection: vi.fn(async () => ({ ok: true })),
    readArtifactSource: () => mockReadSource(),
    saveArtifactUserVersion: (p: unknown) => mockSaveUser(p)
  },
  writable: true,
  configurable: true
})

const { useAppStore } = await import('../src/renderer/stores')
const { parsePersistedBrowsers } = await import('../src/renderer/stores/ui-slice')
const { BrowserCard } = await import('../src/renderer/components/BrowserCard')

const TAB: BrowserTabArtifact = { id: 'a1', version: 3, kind: 'page', title: ART.title }
const url = (n: number): string => `http://127.0.0.1:9000/artifact/a1/${n}?t=tok`

function seed(): void {
  const terminals = new Map()
  terminals.set('t1', {
    id: 't1',
    session: {
      id: 't1',
      projectName: 'repo',
      projectPath: '/repo',
      agentType: 'claude',
      createdAt: 0,
      displayName: 't1'
    },
    status: 'idle',
    lastOutputTimestamp: 1
  })
  act(() => {
    useAppStore.setState({
      terminals,
      browserPanes: new Map(),
      browserMemory: new Map(),
      terminalOrder: ['t1']
    })
  })
}

beforeEach(() => {
  vi.clearAllMocks()
  localStorage.clear()
  mockSelection.mockResolvedValue(null)
  mockPaint.mockResolvedValue({ found: {} })
  artifactState = {
    artifact: ART,
    versions: [version(1), version(2), version(3), version(4, { answersBatchId: 'b1' })],
    comments: [],
    queued: false
  }
  seed()
})

describe('artifact tabs in the store', () => {
  it('adds a tab for an artifact beside the page already open, and reuses it on the next version', () => {
    const s = useAppStore.getState()
    act(() => s.openBrowserPane('t1', 'localhost:5173'))
    act(() => useAppStore.getState().openArtifactTab('t1', url(3), TAB))
    let pane = useAppStore.getState().browserPanes.get('t1')!
    expect(pane.tabs).toHaveLength(2)
    expect(pane.activeTab).toBe(1)
    expect(pane.tabs[0].url).toBe('http://localhost:5173/')

    act(() => useAppStore.getState().setActiveBrowserTab('t1', 0))
    act(() => useAppStore.getState().openArtifactTab('t1', url(4), { ...TAB, version: 4 }))
    pane = useAppStore.getState().browserPanes.get('t1')!
    expect(pane.tabs).toHaveLength(2)
    expect(pane.activeTab).toBe(1)
    expect(pane.tabs[1]).toEqual({ url: url(4), artifact: { ...TAB, version: 4 } })
  })

  it('keeps the artifact when the guest reports where it is', () => {
    act(() => useAppStore.getState().openArtifactTab('t1', url(3), TAB))
    act(() => useAppStore.getState().syncBrowserTab('t1', 0, { url: url(3), title: 'x' }))
    expect(useAppStore.getState().browserPanes.get('t1')!.tabs[0].artifact).toEqual(TAB)
  })

  it('survives a restart, and drops an artifact record that is not one', () => {
    const restored = parsePersistedBrowsers({
      t1: {
        tabs: [
          { url: url(3), artifact: TAB },
          { url: 'https://vorn.dev/', artifact: { id: 'x' } as never }
        ],
        activeTab: 0,
        sessionId: 't1'
      }
    })
    expect(restored.get('t1')!.tabs).toEqual([
      { url: url(3), artifact: TAB },
      { url: 'https://vorn.dev/' }
    ])
  })
})

describe('artifact tab in the browser pane', () => {
  it('names the tab by title and swaps the address bar for the version bar', async () => {
    act(() => useAppStore.getState().openArtifactTab('t1', url(3), TAB))
    render(<BrowserCard sessionId="t1" />)

    expect(screen.getByRole('tab')).toHaveTextContent('Figures for the triage article')
    expect(screen.queryByLabelText('Address')).toBeNull()
    expect(screen.getByRole('button', { name: /Version 3/ })).toBeTruthy()
    await waitFor(() => expect(screen.getByText(/claude · just now/)).toBeTruthy())
  })

  it('lists every version and moves the tab to the one chosen', async () => {
    act(() => useAppStore.getState().openArtifactTab('t1', url(3), TAB))
    render(<BrowserCard sessionId="t1" />)
    await waitFor(() => expect(screen.getByText(/claude · just now/)).toBeTruthy())

    fireEvent.click(screen.getByRole('button', { name: /Version 3/ }))
    const items = screen.getAllByRole('menuitem')
    expect(items.map((i) => i.textContent?.slice(0, 2))).toEqual(['v4', 'v3', 'v2', 'v1'])
    fireEvent.click(items[3])
    await waitFor(() =>
      expect(useAppStore.getState().browserPanes.get('t1')!.tabs[0]).toEqual({
        url: url(1),
        artifact: { ...TAB, version: 1 }
      })
    )
  })

  it('says a newer version is out, and says what a version answered', async () => {
    artifactState!.comments = [
      comment('c1', { state: 'sent', batchId: 'b1' }),
      comment('c2', { state: 'sent', batchId: 'b1' })
    ]
    act(() => useAppStore.getState().openArtifactTab('t1', url(3), TAB))
    const { unmount } = render(<BrowserCard sessionId="t1" />)
    await waitFor(() => expect(screen.getByText('v4 is out; you are reading v3.')).toBeTruthy())
    unmount()

    act(() => useAppStore.getState().openArtifactTab('t1', url(4), { ...TAB, version: 4 }))
    render(<BrowserCard sessionId="t1" />)
    await waitFor(() => expect(screen.getByText('v4 answers your 2 comments on v3.')).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Compare with v3' }))
    await waitFor(() => expect(document.querySelectorAll('webview')).toHaveLength(2))
    expect(document.querySelector('webview')?.getAttribute('src')).toBe(url(3))
  })

  it('asks for a fresh address once, since the port in the saved one may be gone', async () => {
    act(() =>
      useAppStore.getState().openArtifactTab('t1', 'http://127.0.0.1:1111/artifact/a1/3?t=tok', TAB)
    )
    render(<BrowserCard sessionId="t1" />)
    await waitFor(() =>
      expect(useAppStore.getState().browserPanes.get('t1')!.tabs[0].url).toBe(url(3))
    )
    expect(mockVersionUrl).toHaveBeenCalledTimes(1)
  })
})

describe('commenting on an artifact', () => {
  it('opens a comment on the selected words and saves it as a draft on this version', async () => {
    const anchor = { kind: 'quote' as const, quote: 'two API models', prefix: 'and ', suffix: ',' }
    mockSelection.mockResolvedValue({ anchor, rect: { x: 40, y: 60, width: 90, height: 16 } })
    act(() => useAppStore.getState().openArtifactTab('t1', url(3), TAB))
    render(<BrowserCard sessionId="t1" />)

    fireEvent.click(screen.getByRole('button', { name: 'Comment on the page' }))
    expect(screen.getByRole('complementary', { name: 'Comments' })).toHaveTextContent(
      'Select words on the page'
    )
    const box = await screen.findByLabelText('Comment', {}, { timeout: 2000 })
    fireEvent.change(box, { target: { value: 'Name them.' } })
    fireEvent.click(screen.getByRole('button', { name: 'Add' }))

    expect(mockSave).toHaveBeenCalledWith({
      artifactId: 'a1',
      version: 3,
      anchor,
      body: 'Name them.'
    })
    expect(window.api.clearArtifactSelection).toHaveBeenCalledWith('t1')
  })

  it('lists drafts, flags ones whose words are gone, and sends them together', async () => {
    artifactState!.comments = [comment('c1'), comment('c2', { anchor: null, body: 'General.' })]
    mockPaint.mockResolvedValue({ found: { c1: false } })
    act(() => useAppStore.getState().openArtifactTab('t1', url(3), TAB))
    render(<BrowserCard sessionId="t1" />)
    await waitFor(() => expect(screen.getByRole('button', { name: /Send to agent/ })).toBeEnabled())

    fireEvent.click(screen.getByRole('button', { name: 'Comment on the page' }))
    const rail = screen.getByRole('complementary', { name: 'Comments' })
    expect(rail).toHaveTextContent('two API models')
    expect(rail).toHaveTextContent('The whole version')
    await waitFor(() => expect(rail).toHaveTextContent('words changed'))

    fireEvent.click(screen.getByRole('button', { name: 'Send 2 comments to claude' }))
    await waitFor(() => expect(mockSend).toHaveBeenCalledTimes(1))
  })

  it('says a batch is queued until the agent is back at its prompt', async () => {
    artifactState!.comments = [comment('c1')]
    artifactState!.queued = true
    act(() => useAppStore.getState().openArtifactTab('t1', url(3), TAB))
    render(<BrowserCard sessionId="t1" />)
    await waitFor(() => expect(screen.getByRole('button', { name: /Queued/ })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'Comment on the page' }))
    expect(screen.getByText(/Queued\. It goes once claude is back at its prompt\./)).toBeTruthy()
  })
})

describe('editing a doc', () => {
  const DOC_TAB: BrowserTabArtifact = { id: 'a1', version: 4, kind: 'doc', title: 'Triage' }

  it('offers Edit only on the latest version', async () => {
    artifactState!.artifact = { ...ART, kind: 'doc' }
    act(() => useAppStore.getState().openArtifactTab('t1', url(3), { ...DOC_TAB, version: 3 }))
    render(<BrowserCard sessionId="t1" />)
    await waitFor(() => expect(screen.getByRole('button', { name: 'Edit' })).toBeDisabled())
  })

  it('saves the edit as the next version and sends each changed paragraph', async () => {
    artifactState!.artifact = { ...ART, kind: 'doc' }
    act(() => useAppStore.getState().openArtifactTab('t1', url(4), DOC_TAB))
    render(<BrowserCard sessionId="t1" />)
    await waitFor(() => expect(screen.getByRole('button', { name: 'Edit' })).toBeEnabled())
    fireEvent.click(screen.getByRole('button', { name: 'Edit' }))
    const words = await screen.findByLabelText('Doc words')
    expect(screen.getByText('doc · you are editing')).toBeTruthy()
    const save = screen.getByRole('button', { name: 'Save v5 and send 0 to claude' })
    expect(save).toBeDisabled()

    fireEvent.change(words, { target: { value: '# Triage\n\nA few things kept me wondering.\n' } })
    fireEvent.click(screen.getByRole('button', { name: 'Save v5 and send 1 to claude' }))
    await waitFor(() =>
      expect(mockSaveUser).toHaveBeenCalledWith({
        artifactId: 'a1',
        body: '# Triage\n\nA few things kept me wondering.\n',
        edits: [{ before: 'It always bothered me.', after: 'A few things kept me wondering.' }],
        send: true
      })
    )
    await waitFor(() => expect(screen.queryByLabelText('Doc words')).toBeNull())
    await waitFor(() =>
      expect(useAppStore.getState().browserPanes.get('t1')!.tabs[0].artifact?.version).toBe(5)
    )
  })

  it('leaves the doc as it was on Discard', async () => {
    artifactState!.artifact = { ...ART, kind: 'doc' }
    act(() => useAppStore.getState().openArtifactTab('t1', url(4), DOC_TAB))
    render(<BrowserCard sessionId="t1" />)
    await waitFor(() => expect(screen.getByRole('button', { name: 'Edit' })).toBeEnabled())
    fireEvent.click(screen.getByRole('button', { name: 'Edit' }))
    fireEvent.change(await screen.findByLabelText('Doc words'), { target: { value: 'Gone.' } })
    fireEvent.click(screen.getByRole('button', { name: 'Discard' }))
    expect(screen.queryByLabelText('Doc words')).toBeNull()
    expect(mockSaveUser).not.toHaveBeenCalled()
  })
})
