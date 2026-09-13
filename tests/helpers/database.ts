import Database from 'libsql'

/** Open the database file on its own connection, run one read or write, and close it again. */
export function queryDb<T>(file: string, fn: (d: Database.Database) => T): T {
  const d = new Database(file)
  try {
    return fn(d)
  } finally {
    d.close()
  }
}
