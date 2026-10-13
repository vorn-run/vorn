export interface OnboardingStep {
  id: string
  title: string
  subtitle: string
  icon: string
}

export const ONBOARDING_STEPS: OnboardingStep[] = [
  {
    id: 'agents',
    title: 'Coding Agents',
    subtitle: 'Detect agents & install skills',
    icon: 'Bot'
  },
  {
    id: 'projects',
    title: 'Projects & Tasks',
    subtitle: 'Organize repos, branches & work items',
    icon: 'FolderGit2'
  },
  {
    id: 'sessions',
    title: 'Launch Sessions',
    subtitle: 'Start agents with the right context',
    icon: 'Rocket'
  },
  {
    id: 'workspace',
    title: 'Your Workspace',
    subtitle: 'Grid, tabs & the review panel',
    icon: 'LayoutDashboard'
  },
  {
    id: 'workflows',
    title: 'Workflows & Schedules',
    subtitle: 'Automate multi-agent pipelines',
    icon: 'Workflow'
  },
  {
    id: 'shortcuts',
    title: 'Command Palette & Shortcuts',
    subtitle: 'Find anything instantly',
    icon: 'Search'
  },
  {
    id: 'ready',
    title: "You're Ready",
    subtitle: 'Start building with your agents',
    icon: 'Sparkles'
  }
]
