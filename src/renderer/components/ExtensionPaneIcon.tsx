import { Puzzle } from 'lucide-react'
import { ConnectorIcon } from './ConnectorIcon'
import type { SdkConnectorIcon } from '../../shared/types'

/**
 * The glyph beside a pane's name, wherever the pane is named.
 *
 * Three answers, narrowing: the pane's own glyph, then the extension's, then a
 * neutral mark — a row with no icon at all sits a text-width out of line with
 * every other row in the menu, and reads as a different kind of thing.
 *
 * Drawn through the same component a packaged connector's glyph goes through,
 * so path data is all a pack can contribute here too.
 */
export function ExtensionPaneIcon({
  icon,
  extensionIcon,
  extensionId,
  size = 14,
  className = 'text-gray-500'
}: {
  icon?: SdkConnectorIcon
  extensionIcon?: SdkConnectorIcon
  extensionId: string
  size?: number
  className?: string
}) {
  const drawn = icon ?? extensionIcon
  if (drawn) {
    return (
      <ConnectorIcon connectorId={extensionId} icon={drawn} size={size} className={className} />
    )
  }
  return <Puzzle size={size} className={className} />
}
