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

/** An upstream answer with a failing status, kept so a sign-out reads apart from an outage. */
export class UpstreamStatusError extends Error {
  readonly status: number

  constructor(status: number, message: string) {
    super(message)
    this.name = 'UpstreamStatusError'
    this.status = status
  }
}

/** How deep a chain of wrapped errors is followed before giving up on it. */
const MAX_CAUSES = 8

function* causes(error: unknown): Generator<unknown> {
  let at = error
  for (let depth = 0; at !== undefined && depth < MAX_CAUSES; depth++) {
    yield at
    at = at instanceof Error ? at.cause : undefined
  }
}

/** A failed call as the wire carries it; `signedIn` says the connector's calls go through a Vorn window. */
export function protocolError(error: unknown, signedIn = false): ProtocolError {
  const message = error instanceof Error ? error.message : String(error)
  const failed = (data: ProtocolErrorData): ProtocolError => ({
    code: PROTOCOL_ERROR_CODES.connectorError,
    message,
    data
  })
  for (const at of causes(error)) {
    if (at instanceof ActionArgumentError) return failed({ kind: 'validation', field: at.field })
    if (at instanceof SessionUnavailableError) {
      return failed({ kind: 'app-offline', retryable: false })
    }
    if (at instanceof UpstreamStatusError) {
      if (signedIn && (at.status === 401 || at.status === 403)) {
        return failed({ kind: 'signed-out', retryable: false })
      }
      return failed({ kind: 'upstream', retryable: RETRYABLE_STATUS.has(at.status) })
    }
  }
  return failed({ kind: 'internal' })
}
