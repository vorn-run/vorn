import { AgentStatus, AppConfig, WorkflowDefinition } from '../../shared/types'
import { TerminalState } from '../stores/types'
import { getDisplayName } from './terminal-display'
import { AGENT_DEFINITIONS } from './agent-definitions'

export type NotificationReason = 'waiting' | 'error' | 'bell'

const COOLDOWN_MS = 10_000
const lastNotified = new Map<string, number>()

// --- Web Audio API sound ---

let audioCtx: AudioContext | null = null

function getAudioContext(): AudioContext {
  if (!audioCtx) audioCtx = new AudioContext()
  return audioCtx
}

const SOUND_PROFILES: Record<NotificationReason, { freq: number; duration: number }> = {
  waiting: { freq: 880, duration: 0.15 },
  error: { freq: 330, duration: 0.3 },
  bell: { freq: 660, duration: 0.2 }
}

export function playNotificationSound(reason: NotificationReason, volume: number = 0.5): void {
  const ctx = getAudioContext()
  const profile = SOUND_PROFILES[reason]

  const osc = ctx.createOscillator()
  const gain = ctx.createGain()
  osc.connect(gain)
  gain.connect(ctx.destination)

  osc.frequency.value = profile.freq
  osc.type = 'sine'
  gain.gain.setValueAtTime(Math.max(0.001, volume), ctx.currentTime)
  gain.gain.exponentialRampToValueAtTime(0.001, ctx.currentTime + profile.duration)
  osc.start()
  osc.stop(ctx.currentTime + profile.duration)
}

// --- Notification logic ---

export function shouldNotifyStatus(
  config: AppConfig | null,
  prevStatus: AgentStatus,
  newStatus: AgentStatus
): boolean {
  const prefs = config?.defaults.notifications
  if (!prefs?.enabled) return false

  if (newStatus === 'waiting' && prevStatus !== 'waiting') {
    return prefs.onWaiting !== false
  }
  if (newStatus === 'error' && prevStatus !== 'error') {
    return prefs.onError !== false
  }
  return false
}

export function shouldNotifyBell(config: AppConfig | null): boolean {
  const prefs = config?.defaults.notifications
  return !!prefs?.enabled && prefs.onBell !== false
}

function dispatch(
  reason: NotificationReason,
  cooldownKey: string,
  prefs: AppConfig['defaults']['notifications'] | undefined,
  title: string,
  body: string,
  onClick?: () => void
): void {
  if (prefs?.soundEnabled) {
    playNotificationSound(reason, prefs.soundVolume ?? 0.5)
  }

  if (Notification.permission !== 'granted') return
  if (document.hasFocus()) return

  const lastTime = lastNotified.get(cooldownKey) ?? 0
  if (Date.now() - lastTime < COOLDOWN_MS) return
  lastNotified.set(cooldownKey, Date.now())

  const notification = new Notification(title, { body, silent: false })
  notification.onclick = () => {
    window.focus()
    onClick?.()
  }
}

export function sendAgentNotification(
  terminal: TerminalState,
  reason: NotificationReason,
  config: AppConfig | null,
  onClick?: () => void
): void {
  const prefs = config?.defaults.notifications
  const name = getDisplayName(terminal.session)
  const agentType = terminal.session.agentType
  const agent = agentType === 'shell' ? 'Shell' : AGENT_DEFINITIONS[agentType].displayName

  let title: string
  let body: string

  switch (reason) {
    case 'waiting':
      title = `${agent} needs input`
      body = `${name} is waiting for your response`
      break
    case 'error':
      title = `${agent} error`
      body = `${name} encountered an error`
      break
    case 'bell':
      title = `${agent} notification`
      body = `${name} is requesting your attention`
      break
  }

  dispatch(reason, terminal.id, prefs, title, body, onClick)
}

function dispatchWorkflowWait(
  tag: string,
  title: string,
  body: string,
  config: AppConfig | null,
  onClick?: () => void
): void {
  const prefs = config?.defaults.notifications
  if (!prefs?.enabled || prefs.onWaiting === false) return
  dispatch('waiting', tag, prefs, title, body, onClick)
}

export function sendWorkflowGateNotification(
  workflow: WorkflowDefinition,
  nodeId: string,
  nodeLabel: string,
  message: string | undefined,
  config: AppConfig | null,
  onClick?: () => void
): void {
  dispatchWorkflowWait(
    `workflow-gate:${workflow.id}:${nodeId}`,
    `${workflow.name} · awaiting approval`,
    message || `${nodeLabel} is waiting for your approval`,
    config,
    onClick
  )
}

export function sendWorkflowSignInNotification(
  workflow: WorkflowDefinition,
  nodeId: string,
  nodeLabel: string,
  message: string | undefined,
  config: AppConfig | null,
  onClick?: () => void
): void {
  dispatchWorkflowWait(
    `workflow-sign-in:${workflow.id}:${nodeId}`,
    `${workflow.name} · awaiting sign-in`,
    message || `${nodeLabel} is waiting for you to sign in again`,
    config,
    onClick
  )
}
