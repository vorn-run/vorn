/** Where this round's review page is served: the local server from the desktop, the same origin from the web client. */
export async function gateViewUrl(runId: string, nodeId: string, token: string): Promise<string> {
  const path = `/gate-view/${encodeURIComponent(runId)}/${encodeURIComponent(nodeId)}?t=${encodeURIComponent(token)}`
  if (window.location.pathname.startsWith('/app')) return path
  const { port } = await window.api.getReachableUrls()
  return `http://127.0.0.1:${port}${path}`
}
