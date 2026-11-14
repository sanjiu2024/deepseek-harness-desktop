/**
 * Who holds a path — via the Windows **Restart Manager**.
 *
 * This is the mechanism behind "the file is in use, close these apps", so the
 * answer comes from Windows rather than from a heuristic. No Sysinternals
 * download, no administrator rights for ordinary files.
 *
 * Lives in `lib/` because two callers need it: the `handles` CLI, and the dev
 * launcher's preflight (a locked executable cannot be started, which reads like
 * a build failure).
 */
import { execFileSync } from 'node:child_process'
import { existsSync } from 'node:fs'

/** Embedded C#: Restart Manager session over one path. */
const QUERY_SCRIPT = String.raw`
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
            RmGetList(session, out needed, ref count, null, ref reasons);
            if (needed == 0) return new string[0];
            count = needed;
            var infos = new RM_PROCESS_INFO[count];
            if (RmGetList(session, out needed, ref count, infos, ref reasons) != 0) return new string[0];
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

/**
 * Processes holding `path`, as `{ pid, name, app }`.
 *
 * Returns an empty list when nothing holds it *or* when the query itself fails —
 * the distinction does not matter to a caller that is about to try an operation
 * and report its own error.
 */
export function holders(path) {
  if (!existsSync(path)) return []

  let stdout
  try {
    stdout = execFileSync(
      'powershell',
      ['-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-Command', QUERY_SCRIPT],
      { encoding: 'utf8', env: { ...process.env, DSH_LOCK_PATH: path } },
    )
  } catch {
    return []
  }

  return stdout
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line.includes('|'))
    .map((line) => {
      const [pid, name, app] = line.split('|')
      return { pid: Number(pid), name: name ?? '?', app: app ?? '' }
    })
    .filter((holder) => Number.isSafeInteger(holder.pid) && holder.pid > 0)
}

/** Terminate the given holders, returning the ones that were killed. */
export async function killHolders(targets) {
  const killed = []
  for (const holder of targets) {
    try {
      process.kill(holder.pid)
      killed.push(holder)
    } catch {
      // Already gone, or not ours to kill. Either way the caller retries.
    }
  }
  if (killed.length > 0) await new Promise((resolve) => setTimeout(resolve, 500))
  return killed
}
