import { cleanup } from '@testing-library/react'
import { afterEach } from 'vitest'
import '@testing-library/jest-dom/vitest'

// jsdom has no layout engine, so scrollIntoView does not exist. The component uses
// it to keep the newest message in view; a no-op is the honest stand-in here.
if (typeof Element !== 'undefined' && typeof Element.prototype.scrollIntoView !== 'function') {
  Element.prototype.scrollIntoView = () => {}
}

// React Testing Library only auto-cleans when `globals` is on; this project keeps
// globals off, so unmount between tests explicitly. Without it a later test sees
// the previous render's DOM and "opens the wrong element" failures look mysterious.
afterEach(() => {
  cleanup()
})
