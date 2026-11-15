/**
 * A kernel that starts and then exits on its own.
 *
 * Exists for one assertion: the pool must forget a route when its process is gone. From the
 * UI's side a client whose stdin is closed looks identical to a slow one — the user sees a
 * spinner that never resolves instead of an error.
 *
 * It exits unconditionally: waiting for a request first would make the test depend on the
 * client sending one, which is a different thing to test.
 */
setTimeout(() => process.exit(0), 500)
