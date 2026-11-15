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
  settingsSet: (provider: string, model: string): Promise<SettingsSnapshot> =>
    invoke<SettingsSnapshot>('settings_set', { provider, model }),
  sessionList: (): Promise<SessionSummary[]> => invoke<SessionSummary[]>('session_list'),
  sessionLoad: (id: string): Promise<SessionHistory> => invoke<SessionHistory>('session_load', { id }),
  sessionRename: (id: string, title: string): Promise<void> =>
    invoke<void>('session_rename', { id, title }),
  sessionDelete: (id: string): Promise<void> => invoke<void>('session_delete', { id }),
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
