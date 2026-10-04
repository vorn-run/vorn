// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// The main-thread fallback, as the other renderer tests stub it.
const inThread = vi.hoisted(() => ({
  codeToTokens: vi.fn(() => ({ tokens: [[{ content: 'here', color: '#111' }]] }))
}))
vi.mock('shiki', () => ({
  createHighlighter: async () => ({
    loadLanguage: async () => undefined,
    codeToTokens: inThread.codeToTokens
  }),
  createJavaScriptRegexEngine: () => ({})
}))

/** A stand-in Worker: records what it is sent and answers when told to. */
class FakeWorker {
  static made: FakeWorker[] = []
  onmessage: ((event: MessageEvent) => void) | null = null
  onerror: ((event: Event) => void) | null = null
  sent: { id: number; code: string; lang: string }[] = []
  terminated = false
  constructor(
    public url: URL,
    public options: WorkerOptions
  ) {
    FakeWorker.made.push(this)
  }
  postMessage(message: { id: number; code: string; lang: string }): void {
    this.sent.push(message)
  }
  answer(index: number, data: Record<string, unknown>): void {
    this.onmessage?.({ data: { id: this.sent[index].id, ...data } } as MessageEvent)
  }
  terminate(): void {
    this.terminated = true
  }
}

async function load(): Promise<typeof import('../src/renderer/components/code-editor/shiki')> {
  vi.resetModules()
  return import('../src/renderer/components/code-editor/shiki')
}

beforeEach(() => {
  FakeWorker.made = []
  inThread.codeToTokens.mockClear()
})

afterEach(() => vi.unstubAllGlobals())

describe('highlighting off the main thread', () => {
  it('asks one shared module worker, and resolves each request with its own answer', async () => {
    vi.stubGlobal('Worker', FakeWorker)
    const { highlightCode } = await load()
    const a = highlightCode('const a = 1', 'typescript')
    const b = highlightCode('b = 2', 'python')
    expect(FakeWorker.made).toHaveLength(1)
    const worker = FakeWorker.made[0]
    expect(worker.options).toEqual({ type: 'module' })
    expect(String(worker.url)).toMatch(/shiki\.worker/)
    expect(worker.sent.map((m) => m.lang)).toEqual(['typescript', 'python'])
    worker.answer(1, { tokens: [[{ content: 'b' }]] })
    worker.answer(0, { tokens: [[{ content: 'a', color: '#fff' }]] })
    await expect(a).resolves.toEqual([[{ content: 'a', color: '#fff' }]])
    await expect(b).resolves.toEqual([[{ content: 'b' }]])
    expect(inThread.codeToTokens).not.toHaveBeenCalled()
  })

  it("passes on the worker's error for one request", async () => {
    vi.stubGlobal('Worker', FakeWorker)
    const { highlightCode } = await load()
    const a = highlightCode('x', 'typescript')
    FakeWorker.made[0].answer(0, { error: 'grammar broke' })
    await expect(a).rejects.toThrow('grammar broke')
  })

  it('moves to the main thread when the worker dies, including what it was asked', async () => {
    vi.stubGlobal('Worker', FakeWorker)
    const { highlightCode } = await load()
    const stranded = highlightCode('x', 'typescript')
    const worker = FakeWorker.made[0]
    worker.onerror?.(new Event('error'))
    await expect(stranded).resolves.toEqual([[{ content: 'here', color: '#111' }]])
    expect(worker.terminated).toBe(true)
    await expect(highlightCode('y', 'typescript')).resolves.toEqual([
      [{ content: 'here', color: '#111' }]
    ])
    expect(FakeWorker.made).toHaveLength(1)
  })

  it('stays on the main thread where a worker cannot be made', async () => {
    vi.stubGlobal(
      'Worker',
      class {
        constructor() {
          throw new Error('not allowed')
        }
      }
    )
    const { highlightCode } = await load()
    await expect(highlightCode('x', 'typescript')).resolves.toEqual([
      [{ content: 'here', color: '#111' }]
    ])
  })

  it('stays on the main thread where there are no workers at all', async () => {
    vi.stubGlobal('Worker', undefined)
    const { highlightCode } = await load()
    await expect(highlightCode('x', 'typescript')).resolves.toEqual([
      [{ content: 'here', color: '#111' }]
    ])
  })
})

describe('the worker itself', () => {
  async function startWorker(oniguruma: () => unknown): Promise<{
    ask: (data: { id: number; code: string; lang: string }) => void
    posted: unknown[]
  }> {
    vi.resetModules()
    vi.doMock('shiki/engine/oniguruma', () => ({ createOnigurumaEngine: oniguruma }))
    vi.doMock('shiki/wasm', () => ({ default: {} }))
    const posted: unknown[] = []
    const scope = self as unknown as {
      onmessage: ((e: { data: unknown }) => void) | null
      postMessage: (m: unknown) => void
    }
    vi.spyOn(scope, 'postMessage').mockImplementation((m: unknown) => {
      posted.push(m)
    })
    await import('../src/renderer/components/code-editor/shiki.worker')
    return { ask: (data) => scope.onmessage?.({ data }), posted }
  }

  afterEach(() => {
    vi.doUnmock('shiki/engine/oniguruma')
    vi.doUnmock('shiki/wasm')
    vi.restoreAllMocks()
  })

  it('answers each request by its id', async () => {
    const engine = vi.fn(() => ({}))
    const { ask, posted } = await startWorker(engine)
    ask({ id: 7, code: 'x', lang: 'typescript' })
    await vi.waitFor(() => expect(posted).toHaveLength(1))
    expect(posted[0]).toEqual({ id: 7, tokens: [[{ content: 'here', color: '#111' }]] })
    expect(engine).toHaveBeenCalled()
  })

  it('falls back to the JavaScript engine when WASM cannot start, and still answers', async () => {
    const { ask, posted } = await startWorker(() => {
      throw new Error('WebAssembly.instantiate(): Refused to compile')
    })
    ask({ id: 1, code: 'x', lang: 'typescript' })
    await vi.waitFor(() => expect(posted).toHaveLength(1))
    expect(posted[0]).toEqual({ id: 1, tokens: [[{ content: 'here', color: '#111' }]] })
  })

  it('reports a failure as an error for that id', async () => {
    inThread.codeToTokens.mockImplementationOnce(() => {
      throw new Error('grammar broke')
    })
    const { ask, posted } = await startWorker(() => ({}))
    ask({ id: 3, code: 'x', lang: 'typescript' })
    await vi.waitFor(() => expect(posted).toHaveLength(1))
    expect(posted[0]).toEqual({ id: 3, error: 'grammar broke' })
  })
})
