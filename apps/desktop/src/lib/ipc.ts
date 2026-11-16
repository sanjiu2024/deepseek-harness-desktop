/**
 * The only place the front end talks to Rust.
 *
 * Bindings are written by hand until `tauri-specta` generates them in M1; every
 * type here mirrors a `#[derive(Serialize)]` struct on the Rust side, so a
 * change there without a change here is a compile error rather than a runtime
 * surprise at the boundary.
 */
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

/** Event names, mirrored from `commands.rs`. */
export const KERNEL_EVENT = 'kernel://message'
export const KERNEL_LAGGED_EVENT = 'kernel://lagged'
export const KERNEL_EXIT_EVENT = 'kernel://exit'

export interface AppInfo {
  name: string
  version: string
}

export interface KernelPaths {
  home: string
  kernelDir: string
  entry: string
  fetched: boolean
  /** DSH_DESKTOP_AUTOSTART=1 — start without waiting for a click. */
  autoStart: boolean
}

/** A frame the kernel sent us. */
export type KernelFrame =
  | { kind: 'notification'; method: string; params: unknown }
  | { kind: 'request'; id: FrameId; method: string; params: unknown }
  | { kind: 'response'; id: FrameId; payload: KernelPayload }

/**
 * A JSON-RPC id exactly as the kernel sent it.
 *
 * The kernel uses **string** ids for the requests it issues (`req_<uuid>`) and
 * this client uses numbers, so narrowing to `number` silently turns every
 * approval request into a notification the view ignores.
 */
export type FrameId = number | string

export type KernelPayload =
  | { status: 'ok'; [index: string]: unknown }
  | { status: 'err'; code: number; message: string; data: string | null }

export interface KernelExit {
  code: number | null
  requested: boolean
}

export interface InitializeOptions {
  cwd: string
  provider: string
  model: string
  reasoningEffort?: string
}

/** One row in the session sidebar, read from the kernel's own log store. */
export interface SessionSummary {
  id: string
  title: string
  cwd: string | null
  createdAt: number
  updatedAt: number
  eventCount: number
  isSubagent: boolean
  /** The session that spawned this one, for nesting subagent runs. */
  parentId: string | null
}

/** The event row shape as it appears on disk. */
export interface StoredEvent {
  type: string
  seq: number
  time: number
  data: unknown
}

export interface SessionHistory {
  header: { id: string; cwd?: string; createdAt: number }
  events: StoredEvent[]
}

/** Route settings, plus whether a key exists (never the key itself). */
export interface SettingsSnapshot {
  provider: string
  model: string
  hasCredential: boolean
  /** Session to reopen on launch, when it still exists. */
  lastSessionId: string | null
  /** Root the file tree browses. */
  workspace: string | null
}

/** One row of the workspace tree. */
export interface TreeEntry {
  name: string
  path: string
  kind: 'file' | 'directory' | 'symlink'
  size: number | null
  modified: number | null
}

/** One match inside a session. */
export interface SearchHit {
  sessionId: string
  title: string
  seq: number
  time: number
  snippet: string
}

/** A file preview, with the encoding it was actually in. */
export interface FilePreview {
  text: string
  encoding: 'utf-8' | 'utf-16le' | 'utf-16be' | 'gb18030'
  /** Bytes had to be replaced: readable, not faithful. */
  lossy: boolean
}

/** One directory, capped. `truncated` means the tree is showing a subset. */
export interface TreeListing {
  entries: TreeEntry[]
  /** How many entries the directory actually had, before the cap. */
  total: number
  truncated: boolean
}

export interface TrashEntry {
  id: string
  deletedAt: number
  bytes: number
  path: string
}

export interface SearchOutcome {
  hits: SearchHit[]
  /** 扫描提前结束：命中上限或字节预算。 */
  truncated: boolean
  /** 为回答这个问题读了多少字节。 */
  scannedBytes: number
}

/** 存储占用：会话、回收站、时间跨度。每个字段都是实测值。 */
export interface StorageStats {
  sessions: number
  sessionBytes: number
  trashEntries: number
  trashBytes: number
  newestActivity: number | null
  oldestActivity: number | null
}

export interface ForkOutcome {
  id: string
  source: string
  path: string
}

export const ipc = {
  appInfo: (): Promise<AppInfo> => invoke<AppInfo>('app_info'),
  kernelPaths: (): Promise<KernelPaths> => invoke<KernelPaths>('kernel_paths'),
  kernelStart: (): Promise<void> => invoke<void>('kernel_start'),
  kernelStop: (): Promise<void> => invoke<void>('kernel_stop'),
  /**
   * Point new sessions at another route without restarting the app.
   *
   * A kernel pins its model at initialize, so this starts a second kernel and leaves the
   * first running — sessions already on it keep working.
   */
  kernelRouteSwitch: (provider: string, model: string): Promise<{ provider: string; model: string }> =>
    invoke<{ provider: string; model: string }>('kernel_route_switch', { provider, model }),
  kernelDiagnostics: (): Promise<string[]> => invoke<string[]>('kernel_diagnostics'),
  settingsGet: (): Promise<SettingsSnapshot> => invoke<SettingsSnapshot>('settings_get'),
  settingsRememberSession: (id: string | null): Promise<void> =>
    invoke<void>('settings_remember_session', { id }),
  settingsSetWorkspace: (path: string): Promise<SettingsSnapshot> =>
    invoke<SettingsSnapshot>('settings_set_workspace', { path }),
  /** Store a provider key in provider.env; it never enters this process's environment. */
  settingsSetSecret: (name: string, value: string): Promise<SettingsSnapshot> =>
    invoke<SettingsSnapshot>('settings_set_secret', { name, value }),
  settingsSet: (provider: string, model: string): Promise<SettingsSnapshot> =>
    invoke<SettingsSnapshot>('settings_set', { provider, model }),
  fsTree: (
    path: string | null,
    showNoisy = false,
    order: 'name' | 'size' | 'modified' = 'name',
  ): Promise<TreeListing> => invoke<TreeListing>('fs_tree', { path, showNoisy, order }),
  fsRead: (path: string, maxBytes?: number): Promise<FilePreview> =>
    invoke<FilePreview>('fs_read', { path, maxBytes }),
  ptyOpen: (id: string, rows: number, cols: number): Promise<{ id: string; shell: string }> =>
    invoke<{ id: string; shell: string }>('pty_open', { id, rows, cols }),
  ptyWrite: (id: string, data: string): Promise<void> => invoke<void>('pty_write', { id, data }),
  ptyResize: (id: string, rows: number, cols: number): Promise<void> =>
    invoke<void>('pty_resize', { id, rows, cols }),
  ptyClose: (id: string): Promise<void> => invoke<void>('pty_close', { id }),
  sessionList: (): Promise<SessionSummary[]> => invoke<SessionSummary[]>('session_list'),
  sessionLoad: (id: string): Promise<SessionHistory> => invoke<SessionHistory>('session_load', { id }),
  /** Full-text search across stored sessions; a scan, not an index. */
  sessionSearch: (query: string, limit?: number): Promise<SearchOutcome> =>
    invoke<SearchOutcome>('session_search', { query, limit }),
  sessionRename: (id: string, title: string): Promise<void> =>
    invoke<void>('session_rename', { id, title }),
  /** Move a session to trash/ — recoverable. */
  sessionDelete: (id: string): Promise<void> => invoke<void>('session_delete', { id }),
  /** Remove a session for good. Nothing recovers this. */
  sessionPurge: (id: string): Promise<void> => invoke<void>('session_purge', { id }),
  /** Copy a session into a new one. The copy is resumable by the kernel. */
  sessionFork: (id: string, title?: string): Promise<ForkOutcome> =>
    invoke<ForkOutcome>('session_fork', { id, title }),
  /** What is in the trash, with sizes. */
  sessionStats: (): Promise<StorageStats> => invoke<StorageStats>('session_stats'),
  sessionTrash: (): Promise<TrashEntry[]> => invoke<TrashEntry[]>('session_trash'),
  /** Empty the trash; returns how many were removed. */
  sessionEmptyTrash: (): Promise<number> => invoke<number>('session_empty_trash'),
  sessionExport: (id: string): Promise<string> => invoke<string>('session_export', { id }),
  sessionExportTo: (id: string, path: string): Promise<void> =>
    invoke<void>('session_export_to', { id, path }),
  kernelInitialize: (options: InitializeOptions): Promise<unknown> =>
    invoke<unknown>('kernel_initialize', {
      cwd: options.cwd,
      provider: options.provider,
      model: options.model,
      reasoningEffort: options.reasoningEffort ?? null,
    }),
  /** Answer a kernel-initiated request. Forgetting this stalls the agent. */
  kernelAnswer: (id: FrameId, result: unknown): Promise<void> =>
    invoke<void>('kernel_answer', { id, result }),
}

export function onKernelFrame(handler: (frame: KernelFrame) => void): Promise<UnlistenFn> {
  return listen<KernelFrame>(KERNEL_EVENT, (event) => handler(event.payload))
}

export function onKernelLagged(handler: (skipped: number) => void): Promise<UnlistenFn> {
  return listen<number>(KERNEL_LAGGED_EVENT, (event) => handler(event.payload))
}

export function onKernelExit(handler: (status: KernelExit) => void): Promise<UnlistenFn> {
  return listen<KernelExit>(KERNEL_EXIT_EVENT, (event) => handler(event.payload))
}

/** Call any SDK method. Prefer the typed helpers above when one exists. */
export async function kernelCall<T = unknown>(
  method: string,
  params: unknown,
  timeoutMs?: number,
): Promise<T> {
  return invoke<T>('kernel_call', { method, params, timeoutMs: timeoutMs ?? null })
}

/** Queue one user turn on an SDK session id. */
export function promptSession(sessionId: string, text: string): Promise<{ messageId: string }> {
  return kernelCall('session/prompt', {
    sessionId,
    contentBlocks: [{ type: 'text', text }],
  })
}

/** Terminal output. Base64 because a Tauri event is JSON, and xterm wants bytes. */
export function onPtyData(
  handler: (payload: { id: string; data: string }) => void,
): Promise<UnlistenFn> {
  return listen<{ id: string; data: string }>('pty://data', (event) => handler(event.payload))
}

export function onPtyExit(
  handler: (payload: { id: string; code: number | null }) => void,
): Promise<UnlistenFn> {
  return listen<{ id: string; code: number | null }>('pty://exit', (event) => handler(event.payload))
}
