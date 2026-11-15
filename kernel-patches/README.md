# kernel-patches

Patches applied to the fetched kernel by `scripts/fetch-kernel.mjs`, in filename order.

## Why this directory exists

The kernel is **not in this repository** — it is 819 MB of dependencies, fetched at a pinned
version into `kernel/`, which is git-ignored. That does not mean it cannot be changed. There are
three layers, and the cost rises with each:

| Layer | What it changes | Survives an upgrade | Where it lives |
| --- | --- | --- | --- |
| **Plugin** (`bridge/`) | Adds behaviour: subscribes to events, mounts a service, exposes a route | Yes, until the event shape changes | This repo, mounted through `profiles/sdk/cordis.patch.yml` |
| **Patch** (here) | Changes a line *inside* a kernel module | No — must be rebased | This repo, applied on every fetch |
| **Upstream PR** | Anything that stays patched for more than one release | Yes, permanently | deepseek-ai/deepseek-harness |

Start at the top. A patch is the right tool only when the kernel itself is wrong or missing
something a plugin cannot reach — a hard-coded path, a swallowed error, a missing field on an
existing payload.

## Rules

1. **One concern per file.** `fix-session-title-truncation.patch`, not `misc.patch`.
2. **Name it after what it fixes**, not after the version it was written against.
3. **The header comment says why AND when it can be deleted** — usually "delete when upstream
   ships X".
4. **A failed patch fails the fetch.** That is deliberate: a half-patched kernel is worse than
   an unfetched one, and the failure names the file.
5. **Anything older than one kernel upgrade is a smell.** Either rebase it deliberately, or
   upstream it.

## Format

Ordinary `git diff` output, paths relative to `kernel/`:

```diff
--- a/node_modules/@deepseek-ai/dsh-session-persistence-jsonl/lib/index.js
+++ b/node_modules/@deepseek-ai/dsh-session-persistence-jsonl/lib/index.js
@@ -12,7 +12,7 @@
```

Produce it by editing the fetched file, then:

```bash
cd kernel
git --no-pager diff --no-index --node_modules/.../lib/index.js ./node_modules/.../lib/index.js.orig
```

or, more practically, keep a pristine copy before editing and use `diff -u` with `a/` and `b/`
prefixes so `git apply -p1` accepts it.

## Current patches

None. Everything the client needs so far is a plugin — which is the layer that keeps working.
