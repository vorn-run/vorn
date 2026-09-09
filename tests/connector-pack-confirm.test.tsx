// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest'
import { render, fireEvent, screen } from '@testing-library/react'
import '@testing-library/jest-dom/vitest'
import { PackInstallConfirm } from '../src/renderer/components/settings/PackInstallConfirm'
import type { ConnectorPackSummary } from '../src/shared/types'

const PREVIEW: ConnectorPackSummary = {
  id: 'packdemo',
  name: 'Pack Demo',
  version: '1.1.0',
  token: 'staged-token',
  description: 'Answers back whatever it is asked.',
  triggers: [
    {
      type: 'tick',
      label: 'Tick',
      filters: {
        pollTool: 'poll_tick',
        itemsPath: 'items',
        idField: 'externalId',
        timestampField: 'updatedAt',
        titleField: 'title',
        urlField: 'url',
        cursorArg: 'cursor',
        cursorPath: 'nextCursor'
      }
    }
  ],
  actions: [{ type: 'echo', label: 'Echo' }],
  env: [
    { name: 'API_TOKEN', required: true, secret: true },
    { name: 'REGION', required: false, secret: false }
  ]
}

describe('the sheet shown before a pack is kept', () => {
  it('brings itself into view, since it opens beside the button that raised it', () => {
    const scrollIntoView = vi.fn()
    const had = Object.getOwnPropertyDescriptor(Element.prototype, 'scrollIntoView')
    Element.prototype.scrollIntoView = scrollIntoView
    try {
      render(<PackInstallConfirm preview={PREVIEW} onConfirm={vi.fn()} onCancel={vi.fn()} />)
      expect(scrollIntoView).toHaveBeenCalledWith({ block: 'nearest' })
    } finally {
      if (had) Object.defineProperty(Element.prototype, 'scrollIntoView', had)
      else delete (Element.prototype as { scrollIntoView?: unknown }).scrollIntoView
    }
  })

  it('renders where scrolling an element into view is not implemented', () => {
    const had = Object.getOwnPropertyDescriptor(Element.prototype, 'scrollIntoView')
    delete (Element.prototype as { scrollIntoView?: unknown }).scrollIntoView
    try {
      render(<PackInstallConfirm preview={PREVIEW} onConfirm={vi.fn()} onCancel={vi.fn()} />)
      expect(screen.getByText(/Pack Demo/)).toBeInTheDocument()
    } finally {
      if (had) Object.defineProperty(Element.prototype, 'scrollIntoView', had)
    }
  })

  it('says which variables the pack will borrow from a signed-in tool', () => {
    const borrowing: ConnectorPackSummary = {
      ...PREVIEW,
      auth: {
        rung: 'cli',
        probe: { command: 'glab', args: ['auth', 'status'] },
        borrow: { env: ['GITLAB_TOKEN', 'NOT_DECLARED', 'ANTHROPIC_API_KEY'] }
      },
      env: [
        { name: 'gitlab_token', required: false, secret: true },
        { name: 'ANTHROPIC_API_KEY', required: false, secret: true }
      ]
    }
    render(<PackInstallConfirm preview={borrowing} onConfirm={() => {}} onCancel={() => {}} />)
    expect(screen.getByText('Borrows')).toBeInTheDocument()
    expect(screen.getByText('gitlab_token from glab')).toBeInTheDocument()
    expect(screen.queryByText(/NOT_DECLARED/)).toBeNull()
    // Refused by the server, so not promised here either.
    expect(screen.queryByText(/ANTHROPIC_API_KEY/)).toBeNull()
  })

  it('says what the connector is and what it can do', () => {
    const { getByText } = render(
      <PackInstallConfirm preview={PREVIEW} onConfirm={() => {}} onCancel={() => {}} />
    )

    expect(getByText('Pack Demo')).toBeInTheDocument()
    expect(getByText('v1.1.0')).toBeInTheDocument()
    expect(getByText('Tick')).toBeInTheDocument()
    expect(getByText('Echo')).toBeInTheDocument()
  })

  it('lists only the settings a connector cannot run without', () => {
    const { getByText, queryByText } = render(
      <PackInstallConfirm preview={PREVIEW} onConfirm={() => {}} onCancel={() => {}} />
    )

    expect(getByText('API_TOKEN')).toBeInTheDocument()
    expect(queryByText('REGION')).not.toBeInTheDocument()
  })

  it('offers to install when nothing is on disk yet', () => {
    const onConfirm = vi.fn()
    const { getByText } = render(
      <PackInstallConfirm preview={PREVIEW} onConfirm={onConfirm} onCancel={() => {}} />
    )

    fireEvent.click(getByText('Install'))
    expect(onConfirm).toHaveBeenCalled()
  })

  it('offers to update, naming the version kept for a rollback', () => {
    const { getByText } = render(
      <PackInstallConfirm
        preview={{ ...PREVIEW, installedVersion: '1.0.0' }}
        onConfirm={() => {}}
        onCancel={() => {}}
      />
    )

    expect(getByText('Update')).toBeInTheDocument()
    expect(getByText(/Replaces v1\.0\.0/)).toBeInTheDocument()
  })

  it('lets the decision be refused', () => {
    const onCancel = vi.fn()
    const { getByText } = render(
      <PackInstallConfirm preview={PREVIEW} onConfirm={() => {}} onCancel={onCancel} />
    )

    fireEvent.click(getByText('Cancel'))
    expect(onCancel).toHaveBeenCalled()
  })

  it('cannot be double-submitted while the install runs', () => {
    const onConfirm = vi.fn()
    const { getByText } = render(
      <PackInstallConfirm preview={PREVIEW} busy onConfirm={onConfirm} onCancel={() => {}} />
    )

    fireEvent.click(getByText('Installing…'))
    expect(onConfirm).not.toHaveBeenCalled()
  })

  it('stays silent about capabilities a connector does not have', () => {
    const { queryByText } = render(
      <PackInstallConfirm
        preview={{ ...PREVIEW, triggers: [], actions: [], env: [] }}
        onConfirm={() => {}}
        onCancel={() => {}}
      />
    )

    expect(queryByText('Triggers')).not.toBeInTheDocument()
    expect(queryByText('Actions')).not.toBeInTheDocument()
    expect(queryByText('Needs')).not.toBeInTheDocument()
  })
})

describe('the sheet shown before an extension is kept', () => {
  const EXTENSION: ConnectorPackSummary = {
    id: 'review',
    name: 'Review',
    version: '0.1.0',
    kind: 'extension',
    token: 'staged-review',
    triggers: [],
    actions: [],
    env: [],
    contributes: {
      panes: [{ id: 'report', title: 'Report', web: 'web/report/index.html' }],
      footers: [{ id: 'checks', title: 'Checks', every: 30 }]
    },
    permissions: ['git.read', 'terminal.send', 'card.rename']
  }

  it('says what it would add to a card', () => {
    render(<PackInstallConfirm preview={EXTENSION} onConfirm={vi.fn()} onCancel={vi.fn()} />)
    expect(screen.getByText('Adds')).toBeInTheDocument()
    expect(screen.getByText('Report pane, Checks footer')).toBeInTheDocument()
  })

  // What it may touch is the question this sheet exists to answer.
  it('says what it would reach, under the verb it answers to', () => {
    render(<PackInstallConfirm preview={EXTENSION} onConfirm={vi.fn()} onCancel={vi.fn()} />)
    expect(screen.getByText('Reads')).toBeInTheDocument()
    expect(screen.getByText("the worktree's diff and status")).toBeInTheDocument()
    expect(screen.getByText('Sends')).toBeInTheDocument()
    expect(screen.getByText("text into the session's terminal")).toBeInTheDocument()
    expect(screen.getByText('Renames')).toBeInTheDocument()
    expect(screen.getByText('the session card')).toBeInTheDocument()
  })

  it('stays silent about a verb it asks nothing of', () => {
    render(
      <PackInstallConfirm
        preview={{ ...EXTENSION, permissions: ['git.read'] }}
        onConfirm={vi.fn()}
        onCancel={vi.fn()}
      />
    )
    expect(screen.queryByText('Sends')).not.toBeInTheDocument()
    expect(screen.queryByText('Renames')).not.toBeInTheDocument()
  })

  it('shows no Adds row for a connector, which adds nothing to a card', () => {
    render(<PackInstallConfirm preview={PREVIEW} onConfirm={vi.fn()} onCancel={vi.fn()} />)
    expect(screen.queryByText('Adds')).not.toBeInTheDocument()
    expect(screen.queryByText('Reads')).not.toBeInTheDocument()
  })
})
