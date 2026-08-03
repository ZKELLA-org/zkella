// `node:sqlite` landed in Node 22.5+ (experimental) — after this project's
// pinned `@types/node@^20` was cut, so there's no upstream declaration yet.
// Minimal ambient declaration covering only the API this indexer actually uses.
declare module 'node:sqlite' {
  export class DatabaseSync {
    constructor(path: string)
    exec(sql: string): void
    prepare(sql: string): StatementSync
    close(): void
  }
  export class StatementSync {
    run(...params: unknown[]): { changes: number; lastInsertRowid: number | bigint }
    get(...params: unknown[]): unknown
    all(...params: unknown[]): unknown[]
  }
}
