import { ScheduleLogEntry } from '@vornrun/shared/types'
import {
  addScheduleLogEntry as dbAddEntry,
  getScheduleLogEntries as dbGetEntries,
  clearScheduleLog as dbClear
} from './database'
import log from './logger'

class ScheduleLogManager {
  addEntry(entry: ScheduleLogEntry): void {
    try {
      dbAddEntry(entry)
    } catch (err) {
      log.warn({ err }, '[schedule-log] addEntry failed:')
    }
  }

  getEntries(workflowId?: string): ScheduleLogEntry[] {
    try {
      return dbGetEntries(workflowId)
    } catch (err) {
      log.warn({ err }, '[schedule-log] getEntries failed:')
      return []
    }
  }

  clear(): void {
    try {
      dbClear()
    } catch (err) {
      log.warn({ err }, '[schedule-log] clear failed:')
    }
  }
}

export const scheduleLogManager = new ScheduleLogManager()
