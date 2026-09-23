import type { Artifact } from '@vornrun/shared/types'
import { IPC } from '@vornrun/shared/types'
import { registerMethod } from '../ws-handler'
import { ptyManager } from '../pty-manager'
import { clientRegistry } from '../broadcast'
import { browserBridge } from '../browser-bridge'
import {
  deleteArtifactComment,
  getArtifact,
  getArtifactComment,
  getArtifactToken,
  getDataDir,
  insertArtifactComment,
  listArtifactComments,
  listArtifacts,
  listArtifactVersions,
  sendArtifactDrafts,
  updateArtifactComment
} from '../database'
import {
  artifactPath,
  canSeeArtifact,
  publishArtifact,
  readArtifactSource,
  saveUserVersion,
  type PublishingSession
} from './service'
import { createArtifactDelivery } from './delivery'
import log from '../logger'

function callingSession(sessionId: string): PublishingSession {
  const session = ptyManager.getActiveSessions().find((s) => s.id === sessionId)
  if (!session) throw new Error(`Session not found: ${sessionId}`)
  return {
    id: session.id,
    projectName: session.projectName || null,
    root: session.worktreePath ?? session.projectPath ?? null
  }
}

function visibleTo(sessionId: string, artifactId: string): Artifact {
  const artifact = getArtifact(artifactId)
  if (!artifact || !canSeeArtifact(artifact, callingSession(sessionId))) {
    throw new Error(`No artifact ${artifactId} in this session or project.`)
  }
  return artifact
}

const commentsChanged = (artifactId: string): void =>
  clientRegistry.broadcast(IPC.ARTIFACT_COMMENTS_CHANGED, { artifactId })

const delivery = createArtifactDelivery({
  session: (id) => ptyManager.getActiveSessions().find((s) => s.id === id) ?? null,
  write: (id, data) => ptyManager.writeToPty(id, data),
  sendDrafts: (id) => sendArtifactDrafts(id),
  hasDrafts: (id) => listArtifactComments(id, { state: 'draft' }).length > 0,
  artifact: (id) => getArtifact(id),
  changed: commentsChanged,
  latestAuthor: (id) => listArtifactVersions(id).at(-1)?.author
})

/** Publishing, reading and commenting on artifacts; `port` is the server's, known once it listens. */
export function registerArtifactMethods(port: () => number): void {
  ptyManager.on('client-message', (channel: string, payload: unknown) => {
    if (channel === IPC.SESSION_UPDATED) delivery.statusChanged((payload as { id: string }).id)
  })
  ptyManager.on('session-exit', (session: { id: string }) => delivery.sessionEnded(session.id))

  const loopback = (path: string): string => `http://127.0.0.1:${port()}${path}`

  registerMethod('artifact:publish', async ({ sessionId, open, ...request }) => {
    const outcome = publishArtifact(getDataDir(), callingSession(sessionId), request)
    const url = loopback(outcome.path)
    clientRegistry.broadcast(IPC.ARTIFACT_PUBLISHED, {
      artifact: outcome.artifact,
      version: outcome.version
    })
    let opened = false
    if (open !== false) {
      try {
        const { artifact, version } = outcome
        await browserBridge.request('browser:openPane', {
          sessionId,
          url,
          artifact: {
            id: artifact.id,
            version: version.version,
            kind: artifact.kind,
            title: artifact.title
          }
        })
        opened = true
      } catch (err) {
        log.info({ err }, '[artifacts] published without opening the pane')
      }
    }
    return {
      artifact: outcome.artifact,
      version: outcome.version,
      url,
      answered: outcome.answered,
      opened
    }
  })

  registerMethod('artifact:list', ({ sessionId, projectName, limit }) => {
    if (sessionId) {
      const session = callingSession(sessionId)
      return listArtifacts({ sessionId, projectName: session.projectName ?? undefined }, limit)
    }
    return listArtifacts({ projectName }, limit)
  })

  registerMethod('artifact:get', ({ artifactId }) => {
    const artifact = getArtifact(artifactId)
    if (!artifact) return null
    return {
      artifact,
      versions: listArtifactVersions(artifactId),
      comments: listArtifactComments(artifactId),
      queued: delivery.isQueued(artifactId)
    }
  })

  registerMethod('artifact:versionUrl', ({ artifactId, version }) => {
    const artifact = getArtifact(artifactId)
    const token = getArtifactToken(artifactId)
    const n = version ?? artifact?.latestVersion ?? 0
    if (!artifact || !token || n < 1 || n > artifact.latestVersion) return null
    const path = artifactPath(artifactId, n, token)
    return { path, url: loopback(path) }
  })

  registerMethod('artifact:readComments', ({ sessionId, artifactId, version }) => {
    visibleTo(sessionId, artifactId)
    return listArtifactComments(artifactId, version === undefined ? {} : { version })
  })

  registerMethod('artifact:saveComment', ({ artifactId, version, anchor, body }) => {
    const artifact = getArtifact(artifactId)
    if (!artifact) throw new Error(`Artifact not found: ${artifactId}`)
    if (version < 1 || version > artifact.latestVersion) {
      throw new Error(`Artifact ${artifactId} has no version ${version}`)
    }
    const comment = insertArtifactComment({ artifactId, version, anchor, body })
    commentsChanged(artifactId)
    return comment
  })

  registerMethod('artifact:updateComment', ({ commentId, body, anchor }) => {
    const comment = updateArtifactComment(commentId, { body, anchor })
    if (comment) commentsChanged(comment.artifactId)
    return comment
  })

  registerMethod('artifact:send', ({ artifactId }) => delivery.send(artifactId))

  registerMethod('artifact:readSource', ({ sessionId, artifactId, version }) => {
    if (sessionId) visibleTo(sessionId, artifactId)
    return readArtifactSource(getDataDir(), artifactId, version)
  })

  registerMethod('artifact:saveUserVersion', ({ artifactId, body, edits, send }) => {
    const { version } = saveUserVersion(getDataDir(), artifactId, body, edits)
    const artifact = getArtifact(artifactId)!
    clientRegistry.broadcast(IPC.ARTIFACT_PUBLISHED, { artifact, version })
    commentsChanged(artifactId)
    return { version, sent: send ? delivery.send(artifactId) : null }
  })

  registerMethod('artifact:deleteComment', ({ commentId }) => {
    const artifactId = getArtifactComment(commentId)?.artifactId
    const deleted = deleteArtifactComment(commentId)
    if (deleted && artifactId) commentsChanged(artifactId)
    return { deleted }
  })
}
