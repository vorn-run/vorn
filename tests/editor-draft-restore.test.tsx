// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render, screen, act, fireEvent, waitFor } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import type { FileStamp } from '../src/shared/types'

{
  const store = new Map<string, string>()
  Object.defineProperty(globalThis, 'localStorage', {
    configurable: true,
    value: {
      getItem: (k: string) => (store.has(k) ? store.get(k)! : null),
      setItem: (k: string, v: string) => store.set(k, String(v)),
      removeItem: (k: string) => void store.delete(k),
      clear: () => store.clear(),
      key: (i: number) => Array.from(store.keys())[i] ?? null,
      get length() {
        return store.size
      }
    }
  })
}

vi.mock('shiki', () => ({
  createHighlighter: async () => ({
    loadLanguage: async () => undefined,
    codeToTokens: () => ({ tokens: [] })
  }),
  createJavaScriptRegexEngine: () => ({})
}))

if (!Element.prototype.scrollIntoView) Element.prototype.scrollIntoView = function () {}

const readFileContent = vi.fn<() => Promise<string | null>>()
const writeFileContent = vi.fn<() => Promise<{ success: boolean; error?: string }>>()
const fileStamp = vi.fn<() => Promise<FileStamp | null>>()

Object.defineProperty(window, 'api', {
  value: {
    listDir: async () => [],
    readFileContent: () => readFileContent(),
    writeFileContent: () => writeFileContent(),
    fileStamp: () => fileStamp()
  },
  writable: true,
  configurable: true
})

const { FileEditorPane } = await import('../src/renderer/components/FileTreeExplorer')

const ON_DISK = 'line one\nline two'
const STAMP: FileStamp = { size: 17, mtimeMs: 1_700_000_000_000 }
/** The same file after something else wrote to it. */
const MOVED: FileStamp = { size: 22, mtimeMs: 1_700_000_009_000 }

const DRAFTS = 'vorn:drafts'
const PANE = 'editor:term-1'
const PATH = '/repo/a.ts'

beforeEach(() => {
  localStorage.clear()
  readFileContent.mockReset().mockResolvedValue(ON_DISK)
  writeFileContent.mockReset().mockResolvedValue({ success: true })
  fileStamp.mockReset().mockResolvedValue(STAMP)
})

async function open(): Promise<void> {
  await act(async () => {
    render(<FileEditorPane cwd="/repo" filePath={PATH} draftKey={PANE} />)
  })
}

function storeDraft(text: string, base: FileStamp | null): void {
  localStorage.setItem(
    DRAFTS,
    JSON.stringify({ [PANE]: { filePath: PATH, text, base, savedAt: Date.now() } })
  )
}

function editor(): HTMLTextAreaElement {
  return screen.getByRole('textbox') as HTMLTextAreaElement
}

describe('an edit that outlived the window', () => {
  it('opens the file back in the editor with the text still in it', async () => {
    storeDraft('line one\nline two edited', STAMP)
    await open()
    await waitFor(() => expect(editor().value).toBe('line one\nline two edited'))
  })

  it('says the file moved while the edit was open, before anything is saved', async () => {
    storeDraft('line one\nline two edited', STAMP)
    fileStamp.mockResolvedValue(MOVED)
    await open()
    expect(await screen.findByText(/changed on disk/i)).toBeInTheDocument()
  })

  it('says nothing when the file is exactly as the edit left it', async () => {
    storeDraft('line one\nline two edited', STAMP)
    await open()
    await waitFor(() => expect(editor()).toBeInTheDocument())
    expect(screen.queryByText(/changed on disk/i)).not.toBeInTheDocument()
  })

  it('lets go of a draft that says the same as the file', async () => {
    storeDraft(ON_DISK, STAMP)
    await open()
    await waitFor(() => expect(screen.queryByRole('textbox')).not.toBeInTheDocument())
    expect(JSON.parse(localStorage.getItem(DRAFTS)!)[PANE]).toBeUndefined()
  })

  it('is not offered to a pane showing a different file', async () => {
    storeDraft('other file entirely', STAMP)
    await act(async () => {
      render(<FileEditorPane cwd="/repo" filePath="/repo/b.ts" draftKey={PANE} />)
    })
    expect(screen.queryByRole('textbox')).not.toBeInTheDocument()
  })
})

describe('saving over a file that moved', () => {
  async function editAndSave(): Promise<void> {
    storeDraft('line one\nline two edited', STAMP)
    await open()
    await waitFor(() => expect(editor()).toBeInTheDocument())
    fileStamp.mockResolvedValue(MOVED)
    await act(async () => {
      fireEvent.click(screen.getByLabelText(/^Save/))
    })
  }

  it('asks instead of overwriting', async () => {
    await editAndSave()
    expect(await screen.findByText(/changed on disk/i)).toBeInTheDocument()
    expect(writeFileContent).not.toHaveBeenCalled()
  })

  it('writes when the person says to save theirs anyway', async () => {
    await editAndSave()
    await act(async () => {
      fireEvent.click(screen.getByText('Save mine anyway'))
    })
    await waitFor(() => expect(writeFileContent).toHaveBeenCalled())
    expect(JSON.parse(localStorage.getItem(DRAFTS)!)[PANE]).toBeUndefined()
  })

  it('takes what is on disk when the person discards theirs', async () => {
    await editAndSave()
    readFileContent.mockResolvedValue('what the agent wrote')
    await act(async () => {
      fireEvent.click(screen.getByText('Discard mine'))
    })
    await waitFor(() => expect(screen.queryByRole('textbox')).not.toBeInTheDocument())
    expect(writeFileContent).not.toHaveBeenCalled()
    expect(JSON.parse(localStorage.getItem(DRAFTS)!)[PANE]).toBeUndefined()
  })

  it('leaves the edit alone when the person wants to keep editing', async () => {
    await editAndSave()
    await act(async () => {
      fireEvent.click(screen.getByText('Keep editing'))
    })
    expect(screen.queryByText(/changed on disk/i)).not.toBeInTheDocument()
    expect(editor().value).toBe('line one\nline two edited')
    expect(JSON.parse(localStorage.getItem(DRAFTS)!)[PANE]).toBeDefined()
  })
})

describe('an unchanged file', () => {
  it('saves without asking', async () => {
    storeDraft('line one\nline two edited', STAMP)
    await open()
    await waitFor(() => expect(editor()).toBeInTheDocument())
    await act(async () => {
      fireEvent.click(screen.getByLabelText(/^Save/))
    })
    await waitFor(() => expect(writeFileContent).toHaveBeenCalled())
    expect(screen.queryByText(/changed on disk/i)).not.toBeInTheDocument()
  })
})
