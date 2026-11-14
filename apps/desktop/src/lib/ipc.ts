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
}

/** A frame the kernel sent us. */
export type KernelFrame =
  | { kind: 'notification'; method: string; params: unknown }
  | { kind: 'request'; id: number; method: string; params: unknown }
  | { kind: 'response'; id: number; payload: KernelPayload }

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

export const ipc = {
  appInfo: (): Promise<AppInfo> => invoke<AppInfo>('app_info'),
  kernelPaths: (): Promise<KernelPaths> => invoke<KernelPaths>('kernel_paths'),
  kernelStart: (): Promise<void> => invoke<void>('kernel_start'),
  kernelStop: (): Promise<void> => invoke<void>('kernel_stop'),
  kernelDiagnostics: (): Promise<string[]> => invoke<string[]>('kernel_diagnostics'),
  kernelInitialize: (options: InitializeOptions): Promise<unknown> =>
    invoke<unknown>('kernel_initialize', {
      cwd: options.cwd,
      provider: options.provider,
      model: options.model,
      reasoningEffort: options.reasoningEffort ?? null,
    }),
  /** Answer a kernel-initiated request. Forgetting this stalls the agent. */
  kernelAnswer: (id: number, result: unknown): Promise<void> =>
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
