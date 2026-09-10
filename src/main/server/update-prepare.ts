import { detachFromServer, stopServer } from './server-launcher'

/** Windows locks a running exe and cannot hand a terminal over, so the server must go first. */
export async function releaseServerForUpdate(platform = process.platform): Promise<void> {
  if (platform === 'win32') await stopServer()
  else detachFromServer()
}
