/**
 * The tool names an SDK connector reserves for Vorn's own use.
 *
 * Mirrors `@vornrun/connector-sdk`'s `setup.ts` rather than importing it: the
 * server would otherwise take a dependency on the SDK's build toolchain for
 * three strings. The names are a wire contract, so they change together or the
 * handshake breaks either way.
 */

/** Tool an SDK connector serves to describe itself. */
export const MANIFEST_TOOL = 'vorn_connector_manifest'

/** Tool a packaged connector registers when it can report its own readiness. */
export const PREFLIGHT_TOOL = 'vorn_connector_preflight'

/** MCP tool name a trigger is served under. */
export function pollToolName(triggerType: string): string {
  return `poll_${triggerType}`
}

/**
 * Whether a tool is plumbing rather than something a workflow step can call.
 *
 * Triggers are matched by the prefix `pollToolName` builds, because a
 * connector's trigger list is not in hand everywhere actions are offered.
 */
export function isReservedSdkTool(name: string): boolean {
  return name === MANIFEST_TOOL || name === PREFLIGHT_TOOL || name.startsWith(pollToolName(''))
}
