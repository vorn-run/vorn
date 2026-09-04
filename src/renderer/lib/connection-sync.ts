/** How long to keep watching for the sync a manual run started. */
const GIVE_UP_AFTER_MS = 15_000

/** How often to ask, slow enough that a poll of a few seconds costs a handful of reads. */
const ASK_EVERY_MS = 500

// `workflow:runManual` answers once the poll is dispatched, so the row watches the connection for the sync it started.
export async function waitForSync(connectionId: string, since: string | undefined): Promise<void> {
  const deadline = Date.now() + GIVE_UP_AFTER_MS
  while (Date.now() < deadline) {
    await new Promise((resolve) => setTimeout(resolve, ASK_EVERY_MS))
    const connections = await window.api.listConnections()
    const syncedAt = connections.find((connection) => connection.id === connectionId)?.lastSyncAt
    // Timestamps are ISO, so they order as text.
    if (syncedAt && (!since || syncedAt > since)) return
  }
}
