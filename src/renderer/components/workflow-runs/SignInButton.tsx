import { LogIn } from 'lucide-react'
import { GATE_APPROVE } from '../../lib/gate-affordance'
import { useConnections } from '../../lib/use-connections'

/** Opens the sign-in window of the connection a parked step acts through. */
export function SignInButton({
  connectionId,
  compact = false
}: {
  connectionId: string | undefined
  compact?: boolean
}) {
  const connections = useConnections()
  const name = connections.find((c) => c.id === connectionId)?.name ?? 'the connection'
  return (
    <button
      type="button"
      disabled={!connectionId}
      onClick={() => connectionId && void window.api.signInConnection(connectionId)}
      className={`flex items-center ${compact ? 'gap-1 px-2 py-1 text-[11px] shrink-0' : 'gap-2 px-4 py-2.5 text-[13px]'} ${GATE_APPROVE} disabled:opacity-50`}
    >
      <LogIn size={compact ? 11 : 14} strokeWidth={compact ? 2.5 : 2} />
      Sign in to {name}
    </button>
  )
}
