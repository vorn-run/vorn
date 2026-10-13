import { isShellSession, type AgentType } from '../../shared/types'

/**
 * Whether to offer a project pane — browser, device, terminals — on a session.
 *
 * These belong to a session that is driving a project. On a plain shell they
 * are noise: a browser and a simulator beside a prompt nobody asked to drive
 * from there, and a panel of shells inside what is already a shell.
 *
 * An open pane always keeps its control, the same exception
 * `shouldShowDeviceButton` makes and for the same reason: hiding the control
 * for a pane that is already on screen takes away the only way to close it.
 *
 * The file tree is deliberately not on this list. Looking at the files next to
 * a shell is exactly as reasonable as looking at them next to an agent.
 */
export function shouldOfferPane(agentType: AgentType | undefined, isOpen: boolean): boolean {
  return isOpen || !isShellSession(agentType)
}
