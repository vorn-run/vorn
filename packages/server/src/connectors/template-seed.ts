import type { WorkflowTemplate } from '@vornrun/shared/types'
import { PORTABLE_FORMAT_VERSION } from '@vornrun/shared/workflow-portability'

/**
 * Enough templates that a first run has somewhere to start.
 *
 * A seed, not a library: these use only built-in steps, so they work on an
 * install with nothing connected, and they are replaced by the published list
 * as soon as one is fetched. Adding to this is not how the catalog grows —
 * publishing is — and a test holds it to that.
 *
 * The webhook trigger ships without a token on purpose. A token published here
 * would be the same secret on every machine that ever used this template, so
 * one is minted when a template becomes a workflow.
 */
export const TEMPLATE_SEED: WorkflowTemplate[] = [
  {
    id: 'webhook-to-report',
    name: 'Webhook to report',
    description:
      'Take a request from another system, look at what it says, and post the ones that matter on somewhere else.',
    steps: ['Webhook', 'Condition', 'HTTP request'],
    category: 'Integrations',
    portable: {
      version: PORTABLE_FORMAT_VERSION,
      slug: 'webhook-to-report',
      name: 'Webhook to report',
      icon: 'Webhook',
      iconColor: '#6f8faf',
      requires: [{ kind: 'httpProfile', nodeId: 'report', name: 'reporting API' }],
      nodes: [
        {
          id: 'trigger',
          type: 'trigger',
          label: 'Webhook',
          config: { triggerType: 'webhook', method: 'POST', token: '' },
          position: { x: 0, y: 0 }
        },
        {
          id: 'check',
          type: 'condition',
          label: 'Condition',
          slug: 'condition',
          config: { variable: '{{trigger.body.severity}}', operator: 'contains', value: 'bug' },
          position: { x: 0, y: 0 }
        },
        {
          id: 'report',
          type: 'httpRequest',
          label: 'HTTP Request',
          slug: 'http-request',
          config: {
            nodeType: 'httpRequest',
            method: 'POST',
            url: '/reports',
            headers: { 'Content-Type': 'application/json' },
            body: '{"summary": "{{trigger.body.summary}}"}'
          },
          position: { x: 0, y: 0 }
        }
      ],
      edges: [
        { id: 'e1', source: 'trigger', target: 'check' },
        { id: 'e2', source: 'check', target: 'report', conditionBranch: 'true' }
      ]
    }
  },
  {
    id: 'morning-digest',
    name: 'Morning digest',
    description:
      'Every weekday morning, gather what changed overnight and have an agent write it up.',
    steps: ['Schedule', 'Script', 'Agent'],
    category: 'Reporting',
    portable: {
      version: PORTABLE_FORMAT_VERSION,
      slug: 'morning-digest',
      name: 'Morning digest',
      icon: 'Newspaper',
      iconColor: '#c9972a',
      nodes: [
        {
          id: 'trigger',
          type: 'trigger',
          label: 'Schedule (Recurring)',
          config: { triggerType: 'recurring', cron: '0 9 * * 1-5' },
          position: { x: 0, y: 0 }
        },
        {
          id: 'gather',
          type: 'script',
          label: 'Execute Script',
          slug: 'execute-script',
          config: {
            scriptType: 'bash',
            projectName: '{{project.name}}',
            projectPath: '{{project.path}}',
            scriptContent: 'git log --since=yesterday --oneline\n'
          },
          position: { x: 0, y: 0 }
        },
        {
          id: 'write',
          type: 'launchAgent',
          label: 'Launch Agent',
          slug: 'launch-agent',
          config: {
            agentType: 'claude',
            projectName: '{{project.name}}',
            projectPath: '{{project.path}}',
            headless: true,
            prompt:
              'Write a short digest of these commits for the team:\n\n{{steps.execute-script.output}}'
          },
          position: { x: 0, y: 0 }
        }
      ],
      edges: [
        { id: 'e1', source: 'trigger', target: 'gather' },
        { id: 'e2', source: 'gather', target: 'write' }
      ]
    }
  },
  {
    id: 'triage-new-tasks',
    name: 'Triage new tasks',
    description: 'When a task is created, have an agent read it and say what it will take.',
    steps: ['Task created', 'Agent'],
    category: 'Tasks',
    portable: {
      version: PORTABLE_FORMAT_VERSION,
      slug: 'triage-new-tasks',
      name: 'Triage new tasks',
      icon: 'ListChecks',
      iconColor: '#7d9471',
      nodes: [
        {
          id: 'trigger',
          type: 'trigger',
          label: 'When Task Created',
          config: { triggerType: 'taskCreated' },
          position: { x: 0, y: 0 }
        },
        {
          id: 'triage',
          type: 'launchAgent',
          label: 'Launch Agent',
          slug: 'launch-agent',
          config: {
            agentType: 'claude',
            projectName: '{{project.name}}',
            projectPath: '{{project.path}}',
            headless: true,
            prompt:
              'Read this task and reply with the files it touches and a rough size:\n\n{{task.title}}\n\n{{task.description}}'
          },
          position: { x: 0, y: 0 }
        }
      ],
      edges: [{ id: 'e1', source: 'trigger', target: 'triage' }]
    }
  },
  {
    id: 'build-from-spec',
    name: 'Build from spec',
    description:
      'Research, build, and review in a loop until the checks pass — then you read it and it opens the PR.',
    steps: ['Manual', 'Agent', 'Agent', 'Loop', 'Approval', 'Script'],
    category: 'Development',
    portable: {
      version: PORTABLE_FORMAT_VERSION,
      slug: 'build-from-spec',
      name: 'Build from spec',
      icon: 'Hammer',
      iconColor: '#8f7a5c',
      nodes: [
        {
          id: 'trigger',
          type: 'trigger',
          label: 'Manual',
          config: {
            triggerType: 'manual',
            inputs: [
              {
                key: 'spec',
                label: 'Spec',
                type: 'textarea',
                required: true,
                placeholder: 'What to build, and what would prove it works.',
                description: 'Every step reads this; nothing else says what the work is.'
              },
              {
                key: 'repoPath',
                label: 'Repository',
                type: 'project',
                required: true,
                description: 'The project the work happens in.'
              },
              {
                key: 'branch',
                label: 'Branch',
                type: 'text',
                required: true,
                placeholder: 'build/what-it-is',
                description: 'Made as a worktree, and pushed under this name.'
              }
            ]
          },
          position: { x: 0, y: 0 }
        },
        {
          id: 'research',
          type: 'launchAgent',
          label: 'Research',
          slug: 'research',
          config: {
            agentType: 'claude',
            projectName: '{{inputs.repoPath}}',
            projectPath: '{{inputs.repoPath.path}}',
            branch: '{{inputs.branch}}',
            useWorktree: true,
            worktreeMode: 'new',
            headless: true,
            timeoutMs: 1_800_000,
            prompt:
              'Read the spec below and research only what it asks, from the published ' +
              'documentation of whatever it names.\n\n' +
              'Write `spec.md` at the root of this worktree: what the work has to achieve, the ' +
              'interfaces it touches, a `## Checks` section naming the command that proves it, ' +
              'and the questions you could not answer. Write that command into `scripts/check.sh` ' +
              'as well, executable. If proving the work needs credentials, write ' +
              '`scripts/check-live.sh` too — and it must exit 0 when those credentials are absent.\n\n' +
              'Do not implement anything yet.\n\n<spec>\n{{inputs.spec}}\n</spec>'
          },
          position: { x: 0, y: 0 }
        },
        {
          id: 'develop',
          type: 'launchAgent',
          label: 'Develop',
          slug: 'develop',
          config: {
            agentType: 'claude',
            projectName: '{{inputs.repoPath}}',
            projectPath: '{{inputs.repoPath.path}}',
            worktreeMode: 'fromStep',
            worktreeFromStepSlug: 'research',
            headless: true,
            timeoutMs: 3_600_000,
            prompt:
              'Implement `spec.md` in this worktree, following the conventions of the repository ' +
              'you are in — read its README and any contributing guide first.\n\n' +
              'Tests alongside the code. Run `scripts/check.sh` until it passes. If a dependency ' +
              'changed, install it and commit the lockfile. Commit as you go.\n\n' +
              '<spec>\n{{inputs.spec}}\n</spec>'
          },
          position: { x: 0, y: 0 }
        },
        {
          id: 'review-loop',
          type: 'loop',
          label: 'Until it is clean',
          slug: 'review-loop',
          config: {
            nodeType: 'loop',
            bodyNodeIds: ['check', 'check-live', 'review'],
            maxIterations: 3,
            until: { variable: '{{steps.review.verdict}}', operator: 'equals', value: 'clean' }
          },
          position: { x: 0, y: 0 }
        },
        {
          id: 'check',
          type: 'script',
          label: 'Check',
          slug: 'check',
          config: {
            scriptType: 'bash',
            cwd: '{{steps.research.worktreePath}}',
            scriptContent:
              'set -o pipefail\n' +
              'if [ ! -x scripts/check.sh ]; then\n' +
              '  echo "scripts/check.sh is missing: the spec has to name the checks" >&2\n' +
              '  exit 1\n' +
              'fi\n' +
              './scripts/check.sh\n'
          },
          position: { x: 0, y: 0 }
        },
        {
          id: 'check-live',
          type: 'script',
          label: 'Check against the real thing',
          slug: 'check-live',
          config: {
            scriptType: 'bash',
            cwd: '{{steps.research.worktreePath}}',
            scriptContent:
              '# Bind a key under "Secrets from" to give this step credentials.\n' +
              'if [ ! -x scripts/check-live.sh ]; then\n' +
              '  echo "no live check"\n' +
              '  exit 0\n' +
              'fi\n' +
              './scripts/check-live.sh\n'
          },
          position: { x: 0, y: 0 }
        },
        {
          id: 'review',
          type: 'launchAgent',
          label: 'Review',
          slug: 'review',
          config: {
            agentType: 'claude',
            projectName: '{{inputs.repoPath}}',
            projectPath: '{{inputs.repoPath.path}}',
            worktreeMode: 'fromStep',
            worktreeFromStepSlug: 'research',
            headless: true,
            timeoutMs: 1_800_000,
            outputSchema: {
              type: 'object',
              required: ['verdict'],
              properties: {
                verdict: {
                  type: 'string',
                  enum: ['clean', 'fix'],
                  description: 'clean only when the checks pass and the spec is met'
                },
                notes: { type: 'string', description: 'What is left, or what you fixed' }
              }
            },
            prompt:
              'Review this worktree against `spec.md`. The checks just ran:\n\n' +
              '<check>\n{{steps.check.output}}\n</check>\n\n' +
              '<live-check>\n{{steps.check-live.output}}\n</live-check>\n\n' +
              'Fix what is wrong and commit it. Answer `clean` only when the checks pass and the ' +
              'spec is met; otherwise answer `fix` and say what is left in `notes`.'
          },
          position: { x: 0, y: 0 }
        },
        {
          id: 'approve',
          type: 'approval',
          label: 'Approval Gate',
          slug: 'approve',
          config: {
            message:
              'Open the worktree the Research step made and read spec.md, the check output, and ' +
              'the reviewer notes. A last verdict of `fix` means the loop ran out of passes. ' +
              'Approve to push the branch and open the PR.'
          },
          position: { x: 0, y: 0 }
        },
        {
          id: 'pr',
          type: 'script',
          label: 'Open the PR',
          slug: 'pr',
          config: {
            scriptType: 'bash',
            cwd: '{{steps.research.worktreePath}}',
            args: ['{{inputs.branch}}', '{{steps.review.notes}}'],
            scriptContent:
              'set -e\n' +
              'git push -u origin "$1"\n' +
              'body=$(mktemp)\n' +
              '{\n' +
              '  cat spec.md\n' +
              '  echo\n' +
              "  echo '## Review'\n" +
              '  printf \'%s\\n\' "$2"\n' +
              '} > "$body"\n' +
              'gh pr create --title "$(git log -1 --pretty=%s)" --body-file "$body"\n'
          },
          position: { x: 0, y: 0 }
        }
      ],
      edges: [
        { id: 'e1', source: 'trigger', target: 'research' },
        { id: 'e2', source: 'research', target: 'develop' },
        { id: 'e3', source: 'develop', target: 'review-loop' },
        { id: 'e4', source: 'review-loop', target: 'check' },
        { id: 'e5', source: 'check', target: 'check-live' },
        { id: 'e6', source: 'check-live', target: 'review' },
        { id: 'e7', source: 'review', target: 'approve' },
        { id: 'e8', source: 'approve', target: 'pr' }
      ]
    }
  }
]
