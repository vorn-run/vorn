/* eslint-disable react-refresh/only-export-components */
import { useState, useEffect, useCallback } from 'react'
import { motion, AnimatePresence } from 'framer-motion'
import { Check, X, AlertTriangle, Info, Loader2 } from 'lucide-react'
import { useIsMobile } from '../hooks/useIsMobile'

/* ------------------------------------------------------------------ */
/*  Toast store — lightweight, no Zustand dependency                  */
/* ------------------------------------------------------------------ */

type ToastType = 'success' | 'error' | 'warning' | 'info' | 'loading'

export interface ToastAction {
  label: string
  onClick: (toastId: string) => void
  tone?: 'default' | 'danger'
}

export interface ToastOptions {
  duration?: number
  actions?: ToastAction[]
}

interface Toast {
  id: string
  message: string
  type: ToastType
  duration: number
  actions?: ToastAction[]
}

const DEFAULT_DURATIONS: Record<ToastType, number> = {
  success: 2500,
  error: 4000,
  warning: 3500,
  info: 2500,
  loading: Number.POSITIVE_INFINITY
}

let listeners: Array<(toasts: Toast[]) => void> = []
let toasts: Toast[] = []
const dismissTimers = new Map<string, ReturnType<typeof setTimeout>>()

function notify() {
  listeners.forEach((fn) => fn([...toasts]))
}

function clearDismissTimer(id: string) {
  const existing = dismissTimers.get(id)
  if (existing) {
    clearTimeout(existing)
    dismissTimers.delete(id)
  }
}

function scheduleDismiss(id: string, duration: number) {
  clearDismissTimer(id)
  if (!Number.isFinite(duration)) return
  const handle = setTimeout(() => {
    dismissTimers.delete(id)
    toasts = toasts.filter((t) => t.id !== id)
    notify()
  }, duration)
  dismissTimers.set(id, handle)
}

export function toast(
  message: string,
  type: ToastType = 'success',
  durationOrOptions?: number | ToastOptions
): string {
  const opts: ToastOptions =
    typeof durationOrOptions === 'number'
      ? { duration: durationOrOptions }
      : (durationOrOptions ?? {})
  const duration = opts.duration ?? DEFAULT_DURATIONS[type]
  const id = crypto.randomUUID()
  toasts = [...toasts, { id, message, type, duration, actions: opts.actions }]
  notify()
  scheduleDismiss(id, duration)
  return id
}

toast.success = (msg: string) => toast(msg, 'success')
toast.error = (msg: string) => toast(msg, 'error', 4000)
toast.warning = (msg: string) => toast(msg, 'warning', 3500)
toast.info = (msg: string) => toast(msg, 'info')
toast.loading = (msg: string) => toast(msg, 'loading')

interface ToastUpdateOptions {
  duration?: number
  actions?: ToastAction[] | null
}

toast.update = (
  id: string,
  message: string,
  type: ToastType,
  opts: ToastUpdateOptions = {}
): string => {
  const existing = toasts.find((t) => t.id === id)
  if (!existing) {
    // Toast was dismissed manually — fall back to a fresh one so feedback isn't lost
    return toast(message, type, {
      duration: opts.duration,
      actions: opts.actions ?? undefined
    })
  }
  const duration = opts.duration ?? DEFAULT_DURATIONS[type]
  const actions = opts.actions === null ? undefined : (opts.actions ?? existing.actions)
  toasts = toasts.map((t) => (t.id === id ? { ...t, message, type, duration, actions } : t))
  notify()
  scheduleDismiss(id, duration)
  return id
}

toast.dismiss = (id: string): void => {
  clearDismissTimer(id)
  toasts = toasts.filter((t) => t.id !== id)
  notify()
}

/**
 * Like `toast.update`, but bails out (returns null) if the toast no longer exists.
 * Use when an async update may race with the user dismissing the toast — without
 * this guard, `toast.update` would resurrect the dismissed toast.
 */
toast.updateIfExists = (
  id: string,
  message: string,
  type: ToastType,
  opts: ToastUpdateOptions = {}
): string | null => {
  const existing = toasts.find((t) => t.id === id)
  if (!existing) return null
  const duration = opts.duration ?? DEFAULT_DURATIONS[type]
  const actions = opts.actions === null ? undefined : (opts.actions ?? existing.actions)
  toasts = toasts.map((t) => (t.id === id ? { ...t, message, type, duration, actions } : t))
  notify()
  scheduleDismiss(id, duration)
  return id
}

/* ------------------------------------------------------------------ */
/*  Icons & colors per type                                           */
/* ------------------------------------------------------------------ */

const TOAST_STYLES: Record<
  ToastType,
  { icon: typeof Check; bg: string; border: string; text: string }
> = {
  success: {
    icon: Check,
    bg: 'bg-green-500/10',
    border: 'border-green-500/20',
    text: 'text-green-400'
  },
  error: {
    icon: X,
    bg: 'bg-red-500/10',
    border: 'border-red-500/20',
    text: 'text-red-400'
  },
  warning: {
    icon: AlertTriangle,
    bg: 'bg-amber-500/10',
    border: 'border-amber-500/20',
    text: 'text-amber-400'
  },
  info: {
    icon: Info,
    bg: 'bg-blue-500/10',
    border: 'border-blue-500/20',
    text: 'text-blue-400'
  },
  loading: {
    icon: Loader2,
    bg: 'bg-white/[0.05]',
    border: 'border-white/10',
    text: 'text-gray-300'
  }
}

/* ------------------------------------------------------------------ */
/*  Toast container — rendered once in App.tsx                        */
/* ------------------------------------------------------------------ */

export function ToastContainer() {
  const [items, setItems] = useState<Toast[]>([])
  const isMobile = useIsMobile()

  useEffect(() => {
    listeners.push(setItems)
    return () => {
      listeners = listeners.filter((l) => l !== setItems)
    }
  }, [])

  const dismiss = useCallback((id: string) => {
    toast.dismiss(id)
  }, [])

  return (
    <div
      className="fixed z-[200] flex flex-col gap-2 pointer-events-none"
      style={{
        bottom: 'calc(1.25rem + var(--safe-bottom, 0px) + var(--keyboard-height, 0px))',
        right: 'calc(1.25rem + var(--safe-right, 0px))'
      }}
    >
      <AnimatePresence mode="popLayout">
        {items.map((t) => {
          const style = TOAST_STYLES[t.type]
          const Icon = style.icon
          return (
            <motion.div
              key={t.id}
              layout
              initial={{ opacity: 0, y: 16, scale: 0.95 }}
              animate={{ opacity: 1, y: 0, scale: 1 }}
              exit={{ opacity: 0, y: -8, scale: 0.95 }}
              transition={{ type: 'spring', stiffness: 400, damping: 28 }}
              className={`pointer-events-auto flex items-center gap-2.5 px-4 py-2.5 rounded-lg border
                         min-w-[200px] max-w-[360px]
                         ${isMobile ? '' : 'shadow-xl backdrop-blur-sm'}
                         ${style.bg} ${style.border}`}
              style={
                isMobile
                  ? {
                      background: 'var(--glass-bg)',
                      backdropFilter: 'var(--glass-blur)',
                      WebkitBackdropFilter: 'var(--glass-blur)',
                      boxShadow: 'var(--glass-shadow)'
                    }
                  : { background: 'rgba(26, 26, 30, 0.92)' }
              }
            >
              <Icon
                size={15}
                strokeWidth={2.5}
                className={`shrink-0 ${style.text} ${t.type === 'loading' ? 'animate-spin' : ''}`}
              />
              <span className="text-sm text-gray-200 flex-1">{t.message}</span>
              {t.actions?.map((action, i) => (
                <button
                  key={i}
                  onClick={() => action.onClick(t.id)}
                  className={`shrink-0 px-2 py-0.5 text-xs rounded transition-colors hover:bg-white/[0.06] ${
                    action.tone === 'danger'
                      ? 'text-red-400 hover:text-red-300'
                      : 'text-gray-300 hover:text-white'
                  }`}
                >
                  {action.label}
                </button>
              ))}
              <button
                onClick={() => dismiss(t.id)}
                className="shrink-0 p-0.5 text-gray-500 hover:text-gray-300 transition-colors"
              >
                <X size={12} strokeWidth={2} />
              </button>
            </motion.div>
          )
        })}
      </AnimatePresence>
    </div>
  )
}
