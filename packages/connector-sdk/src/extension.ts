import type { ConnectorManifest } from './setup'
import type { ActivationPredicate, ConnectorIcon } from './types'

/** What Vorn runs from the folder: the bundled stdio entry, in its folder. */
export const SERVICE_COMMAND = ['node', 'index.js'] as const

/** Re-checks the shipped bundle against the checks `verified.json` records. */
export const CHECKS_COMMAND = ['node', 'index.js', '--check'] as const

/** The name Vorn reads a package's manifest from. */
export const MANIFEST_FILE = 'vorn-extension.toml'

/**
 * The one Vorn extension API a package names. A connector is an extension
 * with a `[connector]` section, whose program speaks the connector protocol
 * that this API version includes.
 */
export const EXTENSION_API = 'vorn:extension@0.9'

/** Every check a receipt may record: the names `check` writes into `verified.json`. */
export const RECORDED_CHECKS = [
  'manifest',
  'auth',
  'secrets',
  'actions',
  'dedupe',
  'no-lifecycle-scripts',
  'keywords',
  'no-runtime-deps',
  'mock',
  'live',
  'launch',
  'contributes',
  'footers',
  'handlers',
  'permissions'
] as const

/** What the manifest says beyond what the connector defines in code. */
export interface ExtensionExtras {
  /** Hosts the connector reaches, `api.example.com` or `*.example.com`; shown at install. */
  hosts?: string[]
  /** Settings, by variable name, whose addresses name hosts it reaches too (a feed list). */
  hostsFrom?: string[]
  /** Who publishes it, shown at install. */
  publisher?: string
}

const BARE_KEY = /^[A-Za-z0-9_-]+$/

/** A TOML basic string: JSON's escapes, which TOML reads alike, plus DEL. */
function str(value: string): string {
  return JSON.stringify(value).replace(/\u007f/g, '\\u007F')
}

function key(name: string): string {
  return BARE_KEY.test(name) ? name : str(name)
}

type Scalar = string | number | boolean
type Inline = Scalar | Inline[] | { [key: string]: Inline | undefined }

function value(v: Inline): string {
  if (typeof v === 'string') return str(v)
  if (typeof v === 'number') return Number.isInteger(v) ? String(v) : String(v)
  if (typeof v === 'boolean') return v ? 'true' : 'false'
  if (Array.isArray(v)) return `[${v.map(value).join(', ')}]`
  const entries = Object.entries(v).filter(([, x]) => x !== undefined) as Array<[string, Inline]>
  return `{ ${entries.map(([k, x]) => `${key(k)} = ${value(x)}`).join(', ')} }`
}

/** The table's lines, its keys in order, absent ones left out. */
function lines(entries: Array<[string, Inline | undefined]>): string[] {
  return entries
    .filter(([, v]) => v !== undefined)
    .map(([k, v]) => `${key(k)} = ${value(v as Inline)}`)
}

function icon(i: ConnectorIcon): Inline {
  return { view_box: i.viewBox, paths: i.paths }
}

function when(w: ActivationPredicate | undefined): Inline | undefined {
  if (w === undefined) return undefined
  return {
    workspace_contains: w.workspaceContains,
    remote_host: w.remoteHost,
    agent: w.agent,
    platform: w.platform
  } as Inline
}

const unless = <T>(v: T, same: T): T | undefined => (v === same ? undefined : v)

/**
 * The connector's `vorn-extension.toml`: the manifest Vorn installs it by,
 * written from what `connectorManifest` reports and what the package says
 * it reaches. Every key is one Vorn's strict reader knows.
 */
export function extensionToml(manifest: ConnectorManifest, extras: ExtensionExtras = {}): string {
  const out: string[] = lines([
    ['id', manifest.id],
    ['name', manifest.name],
    ['version', manifest.version],
    ['api', EXTENSION_API],
    ['publisher', extras.publisher],
    ['description', manifest.description]
  ])
  const table = (header: string, body: string[]): void => {
    if (body.length === 0) return
    out.push('', `[${header}]`, ...body)
  }
  const each = (header: string, body: string[]): void => {
    out.push('', `[[${header}]]`, ...body)
  }
  if (manifest.icon)
    table(
      'icon',
      lines([
        ['view_box', manifest.icon.viewBox],
        ['paths', manifest.icon.paths]
      ])
    )
  table('service', lines([['command', [...SERVICE_COMMAND]]]))
  table('checks', lines([['command', [...CHECKS_COMMAND]]]))
  table(
    'declares',
    lines([
      ['network', extras.hosts?.length ? extras.hosts : undefined],
      ['network_from', extras.hostsFrom?.length ? extras.hostsFrom : undefined],
      ['uses', manifest.permissions?.length ? manifest.permissions : undefined]
    ])
  )
  // The kind section: it names the protocol the program speaks, so it is written even when empty.
  out.push('', '[connector]')
  const auth = manifest.auth
  if (auth) {
    const browser = auth.browser
    table(
      'connector.auth',
      lines([
        ['rung', auth.rung],
        ['probe', auth.probe ? { command: auth.probe.command, args: auth.probe.args } : undefined],
        [
          'borrow',
          auth.borrow
            ? {
                env: auth.borrow.env,
                token_args: auth.borrow.tokenArgs,
                token_env: auth.borrow.tokenEnv
              }
            : undefined
        ],
        ['keys', auth.keys],
        [
          'browser',
          browser
            ? {
                sign_in_url: browser.signInUrl,
                origins: browser.origins,
                check: {
                  url: browser.check.url,
                  identity: browser.check.identity ?? [],
                  headers: browser.check.headers as Inline | undefined
                }
              }
            : undefined
        ]
      ] as Array<[string, Inline | undefined]>)
    )
  }
  for (const t of manifest.triggers) {
    each(
      'connector.triggers',
      lines([
        ['type', t.type],
        ['label', unless(t.label, t.type)],
        ['description', t.description],
        [
          'status_mapping',
          t.statusMapping?.map((s) => ({ upstream: s.upstream, suggested_local: s.suggestedLocal }))
        ],
        [
          'default_workflow',
          t.defaultWorkflow
            ? {
                name: t.defaultWorkflow.name,
                default_cron_from_minutes: t.defaultWorkflow.defaultCronFromMinutes
              }
            : undefined
        ]
      ])
    )
  }
  for (const a of manifest.actions) {
    each(
      'connector.actions',
      lines([
        ['type', a.type],
        ['label', unless(a.label, a.type)],
        ['description', a.description],
        [
          'outputs',
          a.outputs?.map(
            (o) => ({ key: o.key, type: o.type, description: o.description }) as Inline
          )
        ]
      ])
    )
    for (const i of a.inputs) {
      each(
        'connector.actions.inputs',
        lines([
          ['key', i.key],
          ['label', unless(i.label, i.key)],
          ['type', unless(i.type, 'string')],
          ['required', i.required ? true : undefined],
          ['description', i.description],
          ['options', i.options?.map((o) => ({ value: o.value, label: o.label }) as Inline)],
          ['load_options', i.loadOptions]
        ])
      )
    }
  }
  const c = manifest.contributes
  for (const p of c?.panes ?? []) {
    each(
      'contributes.panes',
      lines([
        ['id', p.id],
        ['title', unless(p.title, p.id)],
        ['description', p.description],
        ['when', when(p.when)],
        ['icon', p.icon ? icon(p.icon) : undefined],
        ['web', p.web],
        ['command', p.command]
      ])
    )
  }
  for (const f of c?.footers ?? []) {
    each(
      'contributes.footers',
      lines([
        ['id', f.id],
        ['title', unless(f.title, f.id)],
        ['description', f.description],
        ['when', when(f.when)]
      ]).concat([`every = ${Number.isInteger(f.every) ? `${f.every}.0` : String(f.every)}`])
    )
  }
  for (const l of c?.linkHandlers ?? []) {
    each(
      'contributes.link_handlers',
      lines([
        ['id', l.id],
        ['title', unless(l.title, l.id)],
        ['description', l.description],
        ['when', when(l.when)],
        ['pattern', l.pattern],
        ['example', l.example]
      ])
    )
  }
  if (manifest.activates) {
    const a = when(manifest.activates) as Record<string, Inline | undefined>
    table('activates', lines(Object.entries(a)))
  }
  return `${out.join('\n')}\n`
}

/**
 * What the bundle's `--check` answers: the bundle loads, its manifest is the
 * version `verified.json` recorded, and every check recorded is one this SDK
 * names. The checks themselves ran on the source, where `check` writes the
 * receipt; this holds the shipped bundle to it.
 */
export function selfCheck(
  manifest: Pick<ConnectorManifest, 'id' | 'version'>,
  receipt: unknown,
  known: readonly string[]
): string[] {
  const problems: string[] = []
  const r = receipt as { version?: unknown; checks?: unknown } | undefined
  if (!r || typeof r !== 'object') return ['verified.json is missing or is not an object']
  if (r.version !== manifest.version) {
    problems.push(
      `verified.json records ${String(r.version)}, and the bundle is ${manifest.version}`
    )
  }
  if (!Array.isArray(r.checks) || r.checks.length === 0) {
    problems.push('verified.json records no checks')
  } else {
    for (const check of r.checks) {
      if (!known.includes(String(check)))
        problems.push(`verified.json records an unknown check ${String(check)}`)
    }
  }
  return problems
}
