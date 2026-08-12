/**
 * URL handling for session browser panes.
 *
 * The implementation lives in `src/shared` because the main process needs it
 * too: agent-driven navigation must refuse exactly the schemes the address bar
 * refuses, and two copies of that rule would eventually disagree.
 */
export { normalizeUrl, displayHost } from '../../shared/browser-url'
