import { z } from 'zod'
import type { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import type { Artifact, ArtifactComment, ArtifactVersion } from '@vornrun/shared/types'
import { V } from '../validation'
import { rpcCall } from '@vornrun/server/rpc-client'
import { pageResult, withSession, type ToolResult } from './browser'

const MAX_CONTENT = 5 * 1024 * 1024

const text = (value: string): ToolResult => ({ content: [{ type: 'text', text: value }] })

/** Publishing deliverables into the session's pane, and reading what the person said about them. */
export function registerArtifactTools(server: McpServer): void {
  server.tool(
    'publish_artifact',
    'Publish a page, doc or design for the person to review. It opens in your session browser ' +
      'pane, where they can comment on the exact words and send their notes back to you as one ' +
      'message. Pass artifactId to publish the next version of an artifact after addressing ' +
      'comments. A page or design is self-contained HTML (no network: inline styles, scripts ' +
      'and images as data: URIs); a doc is Markdown.',
    {
      kind: z
        .enum(['page', 'doc', 'design'])
        .describe('page: any HTML; doc: Markdown; design: HTML with a design manifest'),
      title: V.shortText.min(1).describe('What the artifact is called in the pane'),
      file: z
        .string()
        .min(1)
        .max(1000)
        .optional()
        .describe('A .html or .md file inside your project or worktree; give this or content'),
      content: z.string().max(MAX_CONTENT).optional().describe('The HTML or Markdown itself'),
      artifactId: V.id.optional().describe('Publish the next version of this artifact'),
      open: z.boolean().optional().describe('Open it in the browser pane (default true)')
    },
    async (args) =>
      withSession(async (id) => {
        const result = await rpcCall<{
          artifact: Artifact
          version: ArtifactVersion
          url: string
          answered: number
          opened: boolean
        }>('artifact:publish', { sessionId: id, ...args })
        const lines = [
          `Published ${result.artifact.kind} "${result.artifact.title}" v${result.version.version} (artifactId ${result.artifact.id}).`,
          result.opened
            ? 'It is open in your browser pane.'
            : `Not opened in a pane. Served at ${result.url}`
        ]
        if (result.answered > 0) lines.push(`This version answers ${result.answered} comments.`)
        return text(lines.join('\n'))
      })
  )

  server.tool(
    'list_artifacts',
    'List the artifacts published by your session or in your project, newest first.',
    { limit: z.number().int().min(1).max(100).optional() },
    async (args) =>
      withSession(async (id) => {
        const artifacts = await rpcCall<Artifact[]>('artifact:list', {
          sessionId: id,
          limit: args.limit
        })
        if (artifacts.length === 0) return text('No artifacts yet.')
        return text(
          artifacts
            .map(
              (a) => `${a.id}  ${a.kind}  v${a.latestVersion}  "${a.title}"  updated ${a.updatedAt}`
            )
            .join('\n')
        )
      })
  )

  server.tool(
    'read_artifact_comments',
    'Read the comments the person left on an artifact: the quoted words or point each one is ' +
      'about, the comment, which version it was written on, and whether it was sent to you.',
    {
      artifactId: V.id,
      version: z.number().int().min(1).optional().describe('Only comments written on this version')
    },
    async (args) =>
      withSession(async (id) => {
        const comments = await rpcCall<ArtifactComment[]>('artifact:readComments', {
          sessionId: id,
          artifactId: args.artifactId,
          version: args.version
        })
        return pageResult(
          comments.map((c) => ({
            version: c.version,
            state: c.state,
            anchor: c.anchor,
            comment: c.body
          })),
          'ARTIFACT COMMENTS ON WEB PAGE CONTENT'
        )
      })
  )

  server.tool(
    'read_artifact',
    'Read the source of an artifact version: Markdown for a doc, HTML for a page or design. ' +
      'Use it before publishing the next version when the person saved edits of their own.',
    {
      artifactId: V.id,
      version: z.number().int().min(1).optional().describe('Defaults to the latest version')
    },
    async (args) =>
      withSession(async (id) => {
        const found = await rpcCall<{ version: ArtifactVersion; body: string } | null>(
          'artifact:readSource',
          { sessionId: id, artifactId: args.artifactId, version: args.version }
        )
        if (!found) return text(`No such version of artifact ${args.artifactId}.`)
        return pageResult(
          {
            version: found.version.version,
            author: found.version.author === 'user' ? 'the person' : 'an agent',
            source: found.body
          },
          'WEB PAGE CONTENT: ARTIFACT SOURCE'
        )
      })
  )
}
