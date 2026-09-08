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

/** Tool a packaged connector registers when a field's choices need looking up. */
export const OPTIONS_TOOL = 'vorn_connector_options'

/** MCP tool name a trigger is served under. */
export function pollToolName(triggerType: string): string {
  return `poll_${triggerType}`
}

const FOOTER_TOOL_PREFIX = 'vorn_footer_'
const HANDLER_TOOL_PREFIX = 'vorn_handler_'

/** MCP tool name an extension's footer is recomputed under. */
export function footerToolName(footerId: string): string {
  return `${FOOTER_TOOL_PREFIX}${footerId}`
}

/** MCP tool name an extension's link handler is run under. */
export function handlerToolName(handlerId: string): string {
  return `${HANDLER_TOOL_PREFIX}${handlerId}`
}

/**
 * Whether a tool is plumbing rather than something a workflow step can call.
 *
 * Given the connector's trigger types, only their exact poll tools are hidden,
 * so an action a connector genuinely named `poll_status` survives. The prefix
 * is the fallback for callers that cannot say what the triggers are.
 */
export function isReservedSdkTool(name: string, triggerTypes?: readonly string[]): boolean {
  if (name === MANIFEST_TOOL || name === PREFLIGHT_TOOL || name === OPTIONS_TOOL) return true
  // A contribution is called by the app on its own schedule, never by a step.
  if (name.startsWith(FOOTER_TOOL_PREFIX) || name.startsWith(HANDLER_TOOL_PREFIX)) return true
  return triggerTypes
    ? triggerTypes.some((type) => name === pollToolName(type))
    : name.startsWith(pollToolName(''))
}
