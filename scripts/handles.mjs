/**
 * Who is holding this path, and (optionally) kill them.
 *
 * Uses the Windows **Restart Manager** API — the same mechanism as
 * "the file is in use, close these apps" — because it answers exactly the
 * question a leaked handle raises: which process has this file open. It needs no
 * Sysinternals download and no administrator rights for ordinary files.
 *
 * Why this exists: on a machine where unrelated software leaks handles, a build
 * or a browser-profile directory can become permanently "busy" even after the
 * owning window is gone. Deleting the directory then fails with "access denied",
 * which reads like a permissions problem and is not one.
 *
 * Usage:
 *   node scripts/handles.mjs <path>            # report
 *   node scripts/handles.mjs <path> --kill     # terminate the holders
 *   node scripts/handles.mjs <path> --kill --only msedgewebview2,node
 */
import { execFileSync } from 'node:child_process'
import { existsSync } from 'node:fs'

const args = process.argv.slice(2)
const target = args.find((arg) => !arg.startsWith('--'))
const kill = args.includes('--kill')
const onlyAt = args.indexOf('--only')
const only = onlyAt === -1
  ? null
  : (args[onlyAt + 1] ?? '').split(',').map((name) => name.trim().toLowerCase()).filter(Boolean)

if (target === undefined) {
  console.error('usage: node scripts/handles.mjs <path> [--kill] [--only name,name]')
  process.exit(2)
}
if (!existsSync(target)) {
  console.error(`[handles] path does not exist: ${target}`)
  process.exit(2)
}

const script = String.raw`
$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;

public static class RestartManagerQuery {
    [StructLayout(LayoutKind.Sequential)]
    struct RM_UNIQUE_PROCESS {
        public int dwProcessId;
        public System.Runtime.InteropServices.ComTypes.FILETIME ProcessStartTime;
    }

    const int RmRebootReasonNone = 0;
    const int CCH_RM_MAX_APP_NAME = 255;
    const int CCH_RM_MAX_SVC_NAME = 63;

    enum RM_APP_TYPE {
        RmUnknownApp = 0, RmMainWindow = 1, RmOtherWindow = 2, RmService = 3,
        RmExplorer = 4, RmConsole = 5, RmCritical = 1000
    }

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct RM_PROCESS_INFO {
        public RM_UNIQUE_PROCESS Process;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = CCH_RM_MAX_APP_NAME + 1)]
        public string strAppName;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = CCH_RM_MAX_SVC_NAME + 1)]
        public string strServiceShortName;
        public RM_APP_TYPE ApplicationType;
        public uint AppStatus;
        public uint TSSessionId;
        [MarshalAs(UnmanagedType.Bool)]
        public bool bRestartable;
    }

    [DllImport("rstrtmgr.dll", CharSet = CharSet.Unicode)]
    static extern int RmStartSession(out uint pSessionHandle, int dwSessionFlags, string strSessionKey);

    [DllImport("rstrtmgr.dll")]
    static extern int RmEndSession(uint pSessionHandle);

    [DllImport("rstrtmgr.dll", CharSet = CharSet.Unicode)]
    static extern int RmRegisterResources(uint pSessionHandle, uint nFiles, string[] rgsFilenames,
        uint nApplications, RM_UNIQUE_PROCESS[] rgApplications, uint nServices, string[] rgsServiceNames);

    [DllImport("rstrtmgr.dll")]
    static extern int RmGetList(uint dwSessionHandle, out uint pnProcInfoNeeded,
        ref uint pnProcInfo, [In, Out] RM_PROCESS_INFO[] rgAffectedApps, ref uint lpdwRebootReasons);

    public static string[] WhoLocks(string path) {
        uint session;
        var key = Guid.NewGuid().ToString();
        if (RmStartSession(out session, 0, key) != 0) return new string[0];
        try {
            if (RmRegisterResources(session, 1, new[] { path }, 0, null, 0, null) != 0) return new string[0];
            uint needed = 0, count = 0, reasons = 0;
            int result = RmGetList(session, out needed, ref count, null, ref reasons);
            if (needed == 0) return new string[0];
            count = needed;
            var infos = new RM_PROCESS_INFO[count];
            result = RmGetList(session, out needed, ref count, infos, ref reasons);
            if (result != 0) return new string[0];
            var output = new List<string>();
            for (int i = 0; i < count; i++) {
                var info = infos[i];
                string name;
                try { name = System.Diagnostics.Process.GetProcessById(info.Process.dwProcessId).ProcessName; }
                catch { name = "?"; }
                output.Add(info.Process.dwProcessId + "|" + name + "|" + info.strAppName);
            }
            return output.ToArray();
        } finally { RmEndSession(session); }
    }
}
'@
[RestartManagerQuery]::WhoLocks($env:DSH_LOCK_PATH)
`

let stdout
try {
  stdout = execFileSync(
    'powershell',
    ['-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-Command', script],
    { encoding: 'utf8', env: { ...process.env, DSH_LOCK_PATH: target } },
  )
} catch (error) {
  console.error(`[handles] query failed: ${error.message}`)
  process.exit(1)
}

const holders = stdout
  .split(/\r?\n/)
  .map((line) => line.trim())
  .filter((line) => line.includes('|'))
  .map((line) => {
    const [pid, name, app] = line.split('|')
    return { pid: Number(pid), name: name ?? '?', app: app ?? '' }
  })
  .filter((holder) => Number.isSafeInteger(holder.pid) && holder.pid > 0)

if (holders.length === 0) {
  console.log(`[handles] nothing is holding ${target}`)
  process.exit(0)
}

console.log(`[handles] ${holders.length} process(es) hold ${target}`)
for (const holder of holders) {
  console.log(`  pid ${holder.pid}  ${holder.name}  ${holder.app}`)
}

if (!kill) {
  console.log('\nre-run with --kill to terminate them')
  process.exit(0)
}

const targets = only === null
  ? holders
  : holders.filter((holder) => only.includes(holder.name.toLowerCase()))

const skipped = holders.length - targets.length
if (skipped > 0) console.log(`[handles] skipping ${skipped} not in --only`)

for (const holder of targets) {
  try {
    process.kill(holder.pid)
    console.log(`  killed ${holder.pid} ${holder.name}`)
  } catch (error) {
    console.error(`  could not kill ${holder.pid}: ${error.message}`)
  }
}

// Give the OS a moment to drop the handles before the caller retries.
await new Promise((resolve) => setTimeout(resolve, 500))
console.log('[handles] done')
