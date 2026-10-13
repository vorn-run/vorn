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

/**
 * The server this app talks to has been replaced by a different process.
 *
 * Sent after a crash-relaunch: the bridge reconnects on its own, but what it
 * reconnects *to* has none of the PTYs the old one held. Without this the panes
 * carry on showing what they were showing -- their content lives in the
 * renderer, so nothing about them changes -- and they go on accepting input that
 * reaches a terminal which no longer exists.
 *
 * The app's own start-up asks what is running and what is left of what is not.
 * This is the same question, at the only other moment the answer can change out
 * from under it. Not in the `IPC` map for the same reason as above: it is about
 * a server rather than a call to one.
 */
export const SERVER_REPLACED_CHANNEL = 'local-server:replaced'

export interface LocalServerNotice {
  /** How many sessions the local server was holding, when that could be read. */
  sessions: number | null
}

/**
 * Ending the sessions and the server deliberately.
 *
 * The File menu and the command palette both offer this, under the same words,
 * and both reach the one closure in `index.ts` — a label that ends the app from
 * one place and not from another is the kind of difference people only find
 * once.
 */
export const STOP_SESSIONS_AND_SERVER_CHANNEL = 'app:stop-sessions-and-server'
