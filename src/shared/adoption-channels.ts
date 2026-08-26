/**
 * Telling the person that a server is running here when they are looking away.
 *
 * Only one situation needs this channel: the app is pointed at a remote host
 * while a local server is still going. Its sessions keep working and switching
 * back to local reconnects to them — but an agent running on a machine whose app
 * is showing somebody else's is invisible, and invisible is the thing to avoid.
 *
 * The refusal case does *not* come through here. There is no main window then:
 * the app declined to start a second server, so it never got one to render
 * against, and the connect window says it instead.
 *
 * Not in the `IPC` map because that describes the desktop's relay of *server*
 * methods, and this never reaches a server — the whole point is that it is about
 * one this app is not talking to.
 */
export const LOCAL_SERVER_RUNNING_CHANNEL = 'local-server:still-running'

export interface LocalServerNotice {
  /** How many sessions the local server was holding, when that could be read. */
  sessions: number | null
}
