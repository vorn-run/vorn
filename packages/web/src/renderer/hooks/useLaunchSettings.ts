import { useState, useEffect, useRef, useCallback, useMemo } from 'react'
import { useAppStore } from '../stores'
import { AiAgentType } from '../../shared/types'
import { useShallow } from 'zustand/react/shallow'
import { loadLaunchSettings, persistLaunchSettings } from '../lib/launch-prefs'

export type WorktreeMode = 'project-root' | 'existing' | 'new'

export interface WorktreeOption {
  path: string
  branch: string
  name: string
  isMain: boolean
  activeSessionCount: number
}

export function useLaunchSettings() {
  const config = useAppStore((s) => s.config)
  const activeProject = useAppStore((s) => s.activeProject)
  const activeWorktreePath = useAppStore((s) => s.activeWorktreePath)
  const defaultAgent = config?.defaults.defaultAgent || 'claude'

  const [saved] = useState(loadLaunchSettings)
  const [selectedAgent, setSelectedAgent] = useState<AiAgentType>(saved.agent || defaultAgent)
  const [selectedProject, setSelectedProject] = useState(saved.project || '')
  const [localBranches, setLocalBranches] = useState<string[]>([])
  const [remoteBranches, setRemoteBranches] = useState<string[]>([])
  const [currentBranch, setCurrentBranch] = useState<string | null>(null)
  const [selectedBranch, setSelectedBranch] = useState('')
  const [branchFilter, setBranchFilter] = useState('')
  const [showBranchDropdown, setShowBranchDropdown] = useState(false)
  const [loadingBranches, setLoadingBranches] = useState(false)
  const [loadingRemotes, setLoadingRemotes] = useState(false)
  const [worktreeMode, setWorktreeMode] = useState<WorktreeMode>('project-root')
  const [selectedWorktreePath, setSelectedWorktreePath] = useState<string | null>(null)
  const [selectedWorktreeName, setSelectedWorktreeName] = useState<string | null>(null)
  const [liveBranch, setLiveBranch] = useState<string | null>(null)
  const [branchWarning, setBranchWarning] = useState<string | null>(null)
  const [isGitRepo, setIsGitRepo] = useState(true)
  const [existingWorktrees, setExistingWorktrees] = useState<
    { path: string; branch: string; isMain: boolean; name: string }[]
  >([])
  const branchInputRef = useRef<HTMLInputElement>(null)
  const dropdownRef = useRef<HTMLDivElement>(null)

  // Validate saved project exists in config
  useEffect(() => {
    if (selectedProject && config?.projects) {
      const exists = config.projects.some((p) => p.name === selectedProject)
      // eslint-disable-next-line react-hooks/set-state-in-effect -- intentional: clear invalid selection when config changes
      if (!exists) setSelectedProject('')
    }
  }, [config?.projects, selectedProject])

  // Sync with sidebar's active project
  useEffect(() => {
    if (activeProject && config?.projects) {
      const exists = config.projects.some((p) => p.name === activeProject)
      // eslint-disable-next-line react-hooks/set-state-in-effect -- intentional: sync UI selection to sidebar state
      if (exists) setSelectedProject(activeProject)
    }
  }, [activeProject, config?.projects])

  // Sync with sidebar's active worktree
  useEffect(() => {
    if (activeWorktreePath) {
      const wt = existingWorktrees.find((w) => w.path === activeWorktreePath)
      if (wt) {
        // eslint-disable-next-line react-hooks/set-state-in-effect -- intentional: sync UI selection to sidebar state
        setWorktreeMode('existing')
        setSelectedWorktreePath(wt.path)
        setSelectedWorktreeName(wt.name)
      }
    } else {
      setWorktreeMode('project-root')
      setSelectedWorktreePath(null)
      setSelectedWorktreeName(null)
    }
  }, [activeWorktreePath, existingWorktrees])

  // Compute active project path
  const activeProjectPath = config?.projects.find((p) => p.name === selectedProject)?.path || ''

  // Load branches when project changes
  useEffect(() => {
    if (!activeProjectPath) {
      // eslint-disable-next-line react-hooks/set-state-in-effect -- intentional: reset state when project deselected
      setLocalBranches([])
      setRemoteBranches([])
      setCurrentBranch(null)
      setSelectedBranch('')
      setIsGitRepo(true)
      return
    }
    setLoadingBranches(true)
    setRemoteBranches([])
    window.api
      .listBranches(activeProjectPath)
      .then((result) => {
        setLocalBranches(result.local)
        setCurrentBranch(result.current)
        setSelectedBranch(result.current || '')
        setIsGitRepo(result.isGitRepo)
      })
      .catch(() => {
        setLocalBranches([])
        setIsGitRepo(false)
      })
      .finally(() => setLoadingBranches(false))
  }, [activeProjectPath])

  useEffect(() => {
    if (!activeProjectPath) {
      // eslint-disable-next-line react-hooks/set-state-in-effect -- intentional: reset when project deselected
      setExistingWorktrees([])
      return
    }
    window.api
      .listWorktrees(activeProjectPath)
      .then((wts) => setExistingWorktrees(wts.filter((w) => !w.isMain)))
      .catch(() => setExistingWorktrees([]))
  }, [activeProjectPath])

  // Fetch live branch when an existing worktree is selected
  useEffect(() => {
    if (worktreeMode !== 'existing' || !selectedWorktreePath) {
      // eslint-disable-next-line react-hooks/set-state-in-effect -- intentional: clear state when worktree deselected
      setLiveBranch(null)
      setBranchWarning(null)
      return
    }
    window.api
      .getWorktreeBranch(selectedWorktreePath)
      .then((branch) => {
        if (branch) {
          setLiveBranch(branch)
          setSelectedBranch(branch)
        }
      })
      .catch(() => {})
  }, [worktreeMode, selectedWorktreePath])

  // Update branch warning when branch changes on existing worktree
  useEffect(() => {
    if (worktreeMode !== 'existing' || !selectedWorktreePath || !liveBranch) {
      // eslint-disable-next-line react-hooks/set-state-in-effect -- intentional: clear warning when conditions don't apply
      setBranchWarning(null)
      return
    }
    if (selectedBranch === liveBranch) {
      setBranchWarning(null)
      return
    }
    window.api
      .getWorktreeActiveSessions(selectedWorktreePath)
      .then(({ count }) => {
        if (count > 0) {
          setBranchWarning(
            `This will change the branch for ${count} active session${count > 1 ? 's' : ''} in this worktree`
          )
        } else {
          setBranchWarning(null)
        }
      })
      .catch(() => setBranchWarning(null))
  }, [worktreeMode, selectedWorktreePath, selectedBranch, liveBranch])

  // Narrow subscription: only re-render when worktree path counts change
  const worktreePathCounts = useAppStore(
    useShallow((s) => {
      const counts: Record<string, number> = {}
      for (const t of s.terminals.values()) {
        const wp = t.session.worktreePath
        if (wp) counts[wp] = (counts[wp] || 0) + 1
      }
      return counts
    })
  )

  const worktreeOptions: WorktreeOption[] = useMemo(() => {
    return existingWorktrees.map((wt) => ({
      ...wt,
      activeSessionCount: worktreePathCounts[wt.path] || 0
    }))
  }, [existingWorktrees, worktreePathCounts])

  // Close branch dropdown on outside click
  useEffect(() => {
    const handler = (e: MouseEvent): void => {
      if (dropdownRef.current && !dropdownRef.current.contains(e.target as Node)) {
        setShowBranchDropdown(false)
      }
    }
    document.addEventListener('mousedown', handler)
    return () => document.removeEventListener('mousedown', handler)
  }, [])

  const handleFetchRemotes = useCallback(async (): Promise<void> => {
    if (!activeProjectPath || loadingRemotes) return
    setLoadingRemotes(true)
    const remotes = await window.api.listRemoteBranches(activeProjectPath)
    const newRemotes = remotes.filter((r) => !localBranches.includes(r))
    setRemoteBranches(newRemotes)
    setLoadingRemotes(false)
  }, [activeProjectPath, loadingRemotes, localBranches])

  const allBranches = useMemo(
    () => [
      ...localBranches.map((b) => ({ name: b, isRemote: false })),
      ...remoteBranches.map((b) => ({ name: b, isRemote: true }))
    ],
    [localBranches, remoteBranches]
  )

  const filteredBranches = useMemo(
    () =>
      branchFilter
        ? allBranches.filter((b) => b.name.toLowerCase().includes(branchFilter.toLowerCase()))
        : allBranches,
    [allBranches, branchFilter]
  )

  // All projects (no host filtering — remotes are project-level now)
  const filteredProjects = config?.projects ?? []

  const handleProjectChange = useCallback((projectName: string) => {
    setSelectedProject(projectName)
  }, [])

  const handleSelectWorktree = useCallback((wt: WorktreeOption) => {
    setWorktreeMode('existing')
    setSelectedWorktreePath(wt.path)
    setSelectedWorktreeName(wt.name)
  }, [])

  const handleSelectProjectRoot = useCallback(() => {
    setWorktreeMode('project-root')
    setSelectedWorktreePath(null)
    setSelectedWorktreeName(null)
    if (currentBranch) setSelectedBranch(currentBranch)
    setLiveBranch(null)
    setBranchWarning(null)
  }, [currentBranch])

  const handleSelectNewWorktree = useCallback(() => {
    setWorktreeMode('new')
    setSelectedWorktreePath(null)
    setSelectedWorktreeName(null)
    if (currentBranch) setSelectedBranch(currentBranch)
    setLiveBranch(null)
    setBranchWarning(null)
  }, [currentBranch])

  const persist = useCallback(() => {
    persistLaunchSettings({ project: selectedProject, agent: selectedAgent })
  }, [selectedProject, selectedAgent])

  const firstProject = config?.projects?.[0]?.name || ''

  const reset = useCallback(() => {
    const s = loadLaunchSettings()
    setSelectedAgent(s.agent || defaultAgent)
    setSelectedProject(s.project || activeProject || firstProject)
    setRemoteBranches([])
    setBranchFilter('')
    setBranchWarning(null)
    // Restore branch to current and sync worktree from sidebar
    if (currentBranch) setSelectedBranch(currentBranch)
    // Worktree sync effect will handle setting mode from activeWorktreePath
    if (!activeWorktreePath) {
      setWorktreeMode('project-root')
      setSelectedWorktreePath(null)
      setSelectedWorktreeName(null)
      setLiveBranch(null)
    }
  }, [defaultAgent, activeProject, activeWorktreePath, currentBranch, firstProject])

  return {
    selectedAgent,
    setSelectedAgent,
    selectedProject,
    setSelectedProject: handleProjectChange,
    selectedBranch,
    setSelectedBranch,
    branchFilter,
    setBranchFilter,
    showBranchDropdown,
    setShowBranchDropdown,
    loadingBranches,
    loadingRemotes,
    worktreeMode,
    setWorktreeMode,
    selectedWorktreePath,
    setSelectedWorktreePath,
    selectedWorktreeName,
    worktreeOptions,
    localBranches,
    remoteBranches,
    currentBranch,
    activeProjectPath,
    filteredBranches,
    filteredProjects,
    handleFetchRemotes,
    handleSelectWorktree,
    handleSelectProjectRoot,
    handleSelectNewWorktree,
    isGitRepo,
    liveBranch,
    branchWarning,
    branchInputRef,
    dropdownRef,
    persist,
    reset
  }
}
