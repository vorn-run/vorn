import { PROTOCOL_ERROR_CODES, type ProtocolError, type ProtocolErrorData } from './protocol'
import { RETRYABLE_STATUS } from './resilience'
import { SessionUnavailableError } from './session'

/** An action argument that is missing or cannot be read as its declared type. */
export class ActionArgumentError extends Error {
  readonly field: string

  constructor(field: string, message: string) {
    super(message)
    this.name = 'ActionArgumentError'
    this.field = field
  }
}

/** An upstream answer with a failing status; `viaSession` marks one that came through the signed-in window. */
export class UpstreamStatusError extends Error {
  readonly status: number
  readonly viaSession: boolean

  constructor(status: number, message: string, viaSession = false) {
    super(message)
    this.name = 'UpstreamStatusError'
    this.status = status
    this.viaSession = viaSession
  }
}

/** A trigger, action or options set the connector does not have. */
export class UnknownNameError extends Error {
  constructor(message: string) {
    super(message)
    this.name = 'UnknownNameError'
  }
}

export const messageOf = (error: unknown): string =>
  error instanceof Error ? error.message : String(error)

/** An error and each error it wraps, followed eight causes deep at most. */
export function* causes(error: unknown): Generator<unknown> {
  let at = error
  for (let depth = 0; at !== undefined && depth < 8; depth++) {
    yield at
    at = at instanceof Error ? at.cause : undefined
  }
}

/** A failed call as the wire carries it. */
export function protocolError(error: unknown): ProtocolError {
  const failed = (data: ProtocolErrorData): ProtocolError => ({
    code: PROTOCOL_ERROR_CODES.connectorError,
    message: messageOf(error),
    data
  })
  for (const at of causes(error)) {
    if (at instanceof ActionArgumentError) return failed({ kind: 'validation', field: at.field })
    if (at instanceof SessionUnavailableError) {
      return failed({ kind: 'app-offline', retryable: false })
    }
    if (at instanceof UpstreamStatusError) {
      if (at.viaSession && (at.status === 401 || at.status === 403)) {
        return failed({ kind: 'signed-out', retryable: false })
      }
      return failed({ kind: 'upstream', retryable: RETRYABLE_STATUS.has(at.status) })
    }
  }
  return failed({ kind: 'internal' })
}
