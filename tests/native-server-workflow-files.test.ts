/**
 * Workflows as files, through a vornd of the test's own: a project's
 * `.vorn/workflows/*.kdl` is read into its workflows and watched, so an edit
 * to the file changes the workflow; a change made in Vorn is written back to
 * the file with its comments kept; a workflow made in Vorn is saved to a
 * project's folder in one call; a file that does not read says where;
 * `vorn workflow check` names each problem by file, line and column; and a
 * node type installed from git runs, with the workflow its extension ships.
 *
 * Runs where vornd and its session holder have been built.
 */
import { execFileSync, spawnSync } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'
import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest'
import { builtSessiond, builtVornd, servedDir, startServed, type Served } from './helpers/served'
import { Watcher, until } from './helpers/real-server'
import { vornBinary } from './helpers/cli-reference'

vi.setConfig({ testTimeout: 120_000, hookTimeout: 120_000 })

const CREDENTIAL = 'native-server-workflow-files-credential'
const SHOUT = path.resolve(__dirname, '../examples/extensions/shout')

type Json = Record<string, unknown>
type Run = { runId: string; status: string; nodeStates: Array<Json & { nodeId: string }> }

const REVIEW = `// Checks the build, then says so.
workflow "File review" {
    id "file-review"
    enabled #true
    trigger "manual" id="start"
    // The check itself.
    step "script" id="check" slug="check" {
        scriptType "bash"
        scriptContent "echo checked"
    }
    step "script" id="say" slug="say" {
        // Reads the step above.
        scriptType "bash"
        scriptContent "echo said {{steps.check.output}}"
    }
}
`

describe.runIf(!!builtVornd && !!builtSessiond && process.platform !== 'win32')(
  'workflows as files',
  () => {
    let served: Served
    let client: Watcher
    let work = ''
    let folder = ''

    const workflow = (id: string): Promise<Json | null> =>
      client.result<Json | null>('workflow:get', { id })

    async function finished(workflowId: string, count = 1): Promise<Run[]> {
      let runs: Run[] = []
      await until(`${count} finished run(s) of ${workflowId}`, async () => {
        runs = await client.result<Run[]>('workflowRun:list', { workflowId })
        return runs.length >= count && runs.every((r) => r.status !== 'running')
      })
      return runs
    }

    const output = (run: Run, node: string): unknown =>
      run.nodeStates.find((n) => n.nodeId === node)?.output

    beforeAll(async () => {
      served = await startServed({ credential: CREDENTIAL, sessiond: true })
      client = await Watcher.open(served.port, CREDENTIAL)
      work = servedDir('vorn-wf-files-work-')
      folder = path.join(work, '.vorn', 'workflows')
      fs.mkdirSync(folder, { recursive: true })
      fs.writeFileSync(path.join(folder, 'review.kdl'), REVIEW)
      const config = (await client.result('config:load')) as Json
      await client.result('config:save', {
        ...config,
        projects: [{ name: 'p', path: work, preferredAgents: [], workspaceId: 'personal' }]
      })
    })

    afterAll(async () => {
      client?.close()
      await served?.stop()
    })

    it("reads a project's workflow file and runs it", async () => {
      await until('the file to be read', async () => !!(await workflow('file-review')))
      const wf = (await workflow('file-review'))!
      expect(wf.name).toBe('File review')
      expect((wf.nodes as Json[]).map((n) => n.id)).toEqual(['start', 'check', 'say'])
      expect(wf.edges).toEqual([
        { id: 'start-check', source: 'start', target: 'check' },
        { id: 'check-say', source: 'check', target: 'say' }
      ])
      await client.result('workflow:run', { workflowId: 'file-review' })
      const [run] = await finished('file-review')
      expect(run.status).toBe('success')
      expect(output(run, 'say')).toBe('said checked\n')
    })

    it('changes the workflow when its file is edited', async () => {
      const file = path.join(folder, 'review.kdl')
      fs.writeFileSync(file, REVIEW.replace('echo checked', 'echo edited'))
      await until('the edit to be read', async () =>
        JSON.stringify(await workflow('file-review')).includes('echo edited')
      )
      await client.result('workflow:run', { workflowId: 'file-review' })
      const runs = await finished('file-review', 2)
      expect(output(runs[0], 'say')).toBe('said edited\n')
    })

    it('writes a change made in Vorn back to the file, keeping its comments', async () => {
      const file = path.join(folder, 'review.kdl')
      expect(
        await client.result('workflow:update', {
          id: 'file-review',
          updates: { name: 'File review, renamed' }
        })
      ).toEqual({ ok: true })
      await client.result('workflow:setEnabled', { id: 'file-review', enabled: false })
      const text = fs.readFileSync(file, 'utf-8')
      expect(text).toContain('workflow "File review, renamed" {')
      expect(text).toContain('enabled #false')
      for (const kept of [
        '// Checks the build, then says so.',
        '// The check itself.',
        '// Reads the step above.'
      ]) {
        expect(text).toContain(kept)
      }
      // Read back, the file is the workflow as Vorn has it.
      await new Promise((r) => setTimeout(r, 1_500))
      expect((await workflow('file-review'))!.name).toBe('File review, renamed')
    })

    it('lets an edit in the editor and an edit of the file meet without loss', async () => {
      const file = path.join(folder, 'review.kdl')
      const text = fs.readFileSync(file, 'utf-8')
      // The file is edited by hand, and before vornd has read it the editor renames the workflow.
      fs.writeFileSync(file, text.replace('echo edited', 'echo by hand'))
      await client.result('workflow:update', {
        id: 'file-review',
        updates: { name: 'Renamed in the editor' }
      })
      await until('both edits to be in the workflow', async () => {
        const wf = JSON.stringify(await workflow('file-review'))
        return wf.includes('echo by hand') && wf.includes('Renamed in the editor')
      })
      const now = fs.readFileSync(file, 'utf-8')
      expect(now).toContain('echo by hand')
      expect(now).toContain('workflow "Renamed in the editor" {')
      expect(now).toContain('// The check itself.')
    })

    it('saves a workflow made in Vorn to a file in one call', async () => {
      const made = {
        id: 'made-here',
        name: 'Made here',
        icon: 'Zap',
        iconColor: '#ffffff',
        enabled: true,
        nodes: [
          {
            id: 't',
            type: 'trigger',
            label: 'T',
            position: { x: 0, y: 0 },
            config: { triggerType: 'manual' }
          },
          {
            id: 's',
            type: 'script',
            label: 'S',
            position: { x: 0, y: 120 },
            config: { scriptType: 'bash', scriptContent: 'pwd', cwd: path.join(work, 'src') }
          }
        ],
        edges: [{ id: 'e1', source: 't', target: 's' }]
      }
      await client.result('workflow:create', { workflow: made })
      const saved = await client.result<{ path: string; created: boolean }>('workflow:saveToFile', {
        id: 'made-here',
        projectName: 'p'
      })
      expect(saved).toEqual({ path: path.join(folder, 'made-here.kdl'), created: true })
      const text = fs.readFileSync(saved.path, 'utf-8')
      expect(text).toContain('cwd (project)"src"')
      expect(text).toContain('needs="t#e1"')
      const files = await client.result<{ files: Array<{ workflowId: string }> }>('workflow:files')
      expect(files.files.map((f) => f.workflowId).sort()).toEqual(['file-review', 'made-here'])
      const again = await client.result<{ created: boolean }>('workflow:saveToFile', {
        id: 'made-here',
        projectName: 'p'
      })
      expect(again.created).toBe(false)
      await new Promise((r) => setTimeout(r, 1_500))
      const back = (await workflow('made-here'))!
      expect(back.nodes).toEqual(made.nodes)
      expect(back.edges).toEqual(made.edges)
    })

    it('says where a file that does not read goes wrong, and keeps its last workflow', async () => {
      fs.writeFileSync(
        path.join(folder, 'broken.kdl'),
        'workflow "Broken" {\n    id "broken"\n    step "script" id="a" lable="A"\n}\n'
      )
      await until('the error to show', async () => {
        const files = await client.result<{ errors: Array<{ errors: string[] }> }>('workflow:files')
        return files.errors.length > 0
      })
      const files = await client.result<{ errors: Array<{ path: string; errors: string[] }> }>(
        'workflow:files'
      )
      expect(files.errors[0].errors).toEqual([
        `${path.join(folder, 'broken.kdl')}:3:26: unknown property lable=; did you mean label=?`
      ])
      expect(await workflow('broken')).toBeNull()
    })

    it.runIf(!!vornBinary)('checks files from the command line by file, line and column', () => {
      const good = path.join(folder, 'review.kdl')
      const bad = path.join(folder, 'broken.kdl')
      const run = (files: string[]) =>
        spawnSync(vornBinary!, ['workflow', 'check', ...files, '--data-dir', served.dataDir], {
          encoding: 'utf-8',
          env: { ...process.env, HOME: served.home, SECRET_VORN_BOOTSTRAP_TOKEN: CREDENTIAL }
        })
      const ok = run([good])
      expect(ok.status).toBe(0)
      expect(ok.stdout).toMatch(/^1 workflow file read/)
      const wrong = run([good, bad])
      expect(wrong.status).toBe(1)
      expect(wrong.stderr).toContain(`${bad}:3:26: unknown property lable=; did you mean label=?`)
    })

    it('takes a workflow away when its file is removed', async () => {
      fs.rmSync(path.join(folder, 'broken.kdl'))
      fs.rmSync(path.join(folder, 'made-here.kdl'))
      await until('the workflow to go', async () => (await workflow('made-here')) === null)
      expect(await workflow('file-review')).not.toBeNull()
    })

    it('runs a node type installed from git, with the workflow its extension ships', async () => {
      const repo = path.join(work, 'shout-repo')
      fs.cpSync(SHOUT, repo, { recursive: true })
      const env = {
        ...process.env,
        GIT_AUTHOR_NAME: 'Vorn',
        GIT_AUTHOR_EMAIL: 'vorn@example.invalid',
        GIT_COMMITTER_NAME: 'Vorn',
        GIT_COMMITTER_EMAIL: 'vorn@example.invalid'
      }
      for (const args of [
        ['init', '-q', '-b', 'main'],
        ['add', '-A'],
        ['commit', '-q', '-m', 'one']
      ]) {
        execFileSync('git', args, { cwd: repo, env, stdio: 'ignore' })
      }
      const sheet = await client.result<{ token: string }>('extension:inspect', { url: repo })
      await client.result('extension:install', { token: sheet.token, confirm: 'example.shout' })
      await until('its node type to be listed', async () => {
        const types = await client.result<{ nodes: Array<{ id: string }> }>('workflow:nodeTypes')
        return types.nodes.some((n) => n.id === 'shout')
      })
      const types = await client.result<{
        nodes: Array<{ id: string; extension: string; doc: Json }>
      }>('workflow:nodeTypes')
      const shout = types.nodes.find((n) => n.id === 'shout')!
      expect(shout.extension).toBe('example.shout')
      expect(shout.doc.title).toBe('Shout')
      expect((shout.doc.configSchema as Json).required).toEqual(['text'])
      await until(
        'its workflow to be added',
        async () => !!(await workflow('example.shout:try-shout'))
      )
      await client.result('workflow:runManual', {
        workflowId: 'example.shout:try-shout',
        inputs: { words: 'hello' }
      })
      const [run] = await finished('example.shout:try-shout')
      expect(run.status).toBe('success')
      expect(output(run, 'loud')).toBe('HELLO HELLO')
      const described = await client.result<{ nodeTypes: Json }>('workflow:describe')
      expect(Object.keys(described.nodeTypes)).toContain('shout')
    })
  }
)
