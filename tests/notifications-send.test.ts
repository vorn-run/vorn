// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import {
  sendAgentNotification,
  sendWorkflowGateNotification
} from '../src/renderer/lib/notifications'
import type { AppConfig, WorkflowDefinition } from '../src/shared/types'
import type { TerminalState } from '../src/renderer/stores/types'

function makeConfig(overrides: Partial<AppConfig['defaults']['notifications']> = {}): AppConfig {
  return {
    version: 1,
    defaults: {
      shell: '/bin/zsh',
      fontSize: 13,
      theme: 'dark',
      notifications: {
        enabled: true,
        onWaiting: true,
        onError: true,
        onBell: true,
        soundEnabled: false,
        ...overrides
      }
    },
    projects: []
  } as AppConfig
}

function terminal(id: string = `t-${Math.random().toString(36).slice(2, 8)}`): TerminalState {
  return {
    id,
    session: {
      id,
      agentType: 'claude',
      projectName: 'demo',
      projectPath: '/demo',
      cwd: '/demo',
      status: 'idle',
      startedAt: '2026-04-20T10:00:00Z',
      name: 'demo'
    }
  } as unknown as TerminalState
}

function wf(id: string = `wf-${Math.random().toString(36).slice(2, 8)}`): WorkflowDefinition {
  return { ...workflow(), id }
}

function workflow(): WorkflowDefinition {
  return {
    id: 'wf-1',
    name: 'Deploy',
    icon: 'Zap',
    iconColor: 'bronzo',
    enabled: true,
    nodes: [],
    edges: []
  }
}

let notificationInstances: Array<{ onclick?: () => void }> = []
// Declared with the arguments the real constructor takes. Without them the mock
// infers a zero-argument call, and `mock.calls[0]` is an empty tuple -- so every
// assertion about what was dispatched had nothing to read.
const NotificationMock = vi.fn(function (
  this: { onclick?: () => void },
  _title: string,
  _options?: NotificationOptions
) {
  this.onclick = undefined
  notificationInstances.push(this)
})

beforeEach(() => {
  NotificationMock.mockReset()
  notificationInstances = []
  ;(globalThis as unknown as { Notification: unknown }).Notification = Object.assign(
    NotificationMock,
    { permission: 'granted' as const }
  )
  // Force the window to be unfocused so the notification dispatches
  vi.spyOn(document, 'hasFocus').mockReturnValue(false)
})

afterEach(() => {
  vi.restoreAllMocks()
})

describe('sendWorkflowGateNotification', () => {
  it('does nothing when notifications are disabled', () => {
    const config = makeConfig({ enabled: false })
    sendWorkflowGateNotification(wf(), 'n1', 'Gate', 'hi', config)
    expect(NotificationMock).not.toHaveBeenCalled()
  })

  it('does nothing when onWaiting is false', () => {
    const config = makeConfig({ onWaiting: false })
    sendWorkflowGateNotification(wf(), 'n1', 'Gate', 'hi', config)
    expect(NotificationMock).not.toHaveBeenCalled()
  })

  it('dispatches a notification with workflow name and message', () => {
    sendWorkflowGateNotification(wf(), 'n1', 'Gate', 'please review', makeConfig())
    expect(NotificationMock).toHaveBeenCalledTimes(1)
    const [title, opts] = NotificationMock.mock.calls[0]
    expect(title).toContain('Deploy')
    expect((opts as { body: string }).body).toBe('please review')
  })

  it('falls back to node label when message is undefined', () => {
    sendWorkflowGateNotification(wf(), 'n1', 'Approve Deploy', undefined, makeConfig())
    const [, opts] = NotificationMock.mock.calls[0]
    expect((opts as { body: string }).body).toContain('Approve Deploy')
  })

  it('runs onClick when the notification is clicked', () => {
    const onClick = vi.fn()
    sendWorkflowGateNotification(wf(), 'n1', 'Gate', 'hi', makeConfig(), onClick)
    notificationInstances[0]?.onclick?.()
    expect(onClick).toHaveBeenCalled()
  })

  it('keys cooldown by nodeId so distinct nodes with the same label both fire', () => {
    const w = wf()
    sendWorkflowGateNotification(w, 'node-a', 'Gate', 'x', makeConfig())
    sendWorkflowGateNotification(w, 'node-b', 'Gate', 'y', makeConfig())
    expect(NotificationMock).toHaveBeenCalledTimes(2)
  })
})

describe('sendAgentNotification', () => {
  it('dispatches a notification for the waiting reason', () => {
    sendAgentNotification(terminal(), 'waiting', makeConfig())
    expect(NotificationMock).toHaveBeenCalledTimes(1)
    const [title] = NotificationMock.mock.calls[0]
    expect(title).toContain('needs input')
  })

  it('dispatches for the error reason', () => {
    sendAgentNotification(terminal(), 'error', makeConfig())
    expect(NotificationMock).toHaveBeenCalledTimes(1)
    const [title] = NotificationMock.mock.calls[0]
    expect(title).toContain('error')
  })

  it('dispatches for the bell reason', () => {
    sendAgentNotification(terminal(), 'bell', makeConfig())
    expect(NotificationMock).toHaveBeenCalledTimes(1)
    const [title] = NotificationMock.mock.calls[0]
    expect(title).toContain('notification')
  })

  it('does nothing when the window is focused', () => {
    vi.spyOn(document, 'hasFocus').mockReturnValue(true)
    sendAgentNotification(terminal(), 'waiting', makeConfig())
    expect(NotificationMock).not.toHaveBeenCalled()
  })
})
