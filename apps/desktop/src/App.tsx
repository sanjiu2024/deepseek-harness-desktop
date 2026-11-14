import { BootPanel } from '@/features/boot/BootPanel.tsx'

/**
 * Shell. Routing arrives in M1 with the real workspace; for now the only screen
 * is the M0 kernel probe.
 */
export function App() {
  return <BootPanel />
}
