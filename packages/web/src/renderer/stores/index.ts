import { create } from 'zustand'
import { devtools } from 'zustand/middleware'
import { AppStore } from './types'
import { createTerminalsSlice } from './terminals-slice'
import { createProjectsSlice } from './projects-slice'
import { createUISlice } from './ui-slice'
import { createTasksSlice } from './tasks-slice'

export const useAppStore = create<AppStore>()(
  devtools(
    (...a) => ({
      ...createTerminalsSlice(...a),
      ...createProjectsSlice(...a),
      ...createUISlice(...a),
      ...createTasksSlice(...a)
    }),
    { name: 'Vorn' }
  )
)
