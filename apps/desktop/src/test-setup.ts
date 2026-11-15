import '@testing-library/jest-dom/vitest'

// jsdom has no layout engine, so scrollIntoView does not exist. The component uses
// it to keep the newest message in view; a no-op is the honest stand-in here.
if (typeof Element !== 'undefined' && typeof Element.prototype.scrollIntoView !== 'function') {
  Element.prototype.scrollIntoView = () => {}
}
