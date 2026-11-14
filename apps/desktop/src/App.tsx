/**
 * Shell placeholder. The real layout arrives in M1 together with the event
 * reducer; this exists so `pnpm dev` shows something and proves the Vite,
 * Tailwind and React wiring before any kernel work starts.
 */
export function App() {
  return (
    <main className="flex h-full flex-col items-center justify-center gap-3">
      <h1 className="text-2xl font-semibold tracking-tight">DeepSeek Harness Desktop</h1>
      <p className="text-muted-foreground text-sm">
        Skeleton · M0 · the kernel bridge is not wired up yet
      </p>
    </main>
  )
}
