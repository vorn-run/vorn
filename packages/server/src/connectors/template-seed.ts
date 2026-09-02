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
  }
]
