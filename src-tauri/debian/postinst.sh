#!/bin/sh
# Deepseek Harness Desktop —— deb 安装后脚本。
#
# 为什么需要它（离线包的硬约束）：
# 离线包把 Node.js / pnpm / dsh 内核随安装包分发，构建期由
# `.github/actions/prepare-bundle-resources` 把 `resources/manifest.jsonc` 的托管根
# 改写成 `$Resources/{node,pnpm,dsh}`，于是这些运行时落在
# `/usr/lib/<product>/resources` 下 —— 属主是 root、普通用户只读。
# 但桌面端启动时必须写这棵树：
#   * `service::core::runtime::link_required_plugins` 往 `<内核根>/node_modules` 建
#     内置插件入口，写不进去就返回 Err，`prepare_active_runtime` 随即放弃启动；
#   * 启动路径上的 JS 补丁（`service::patch/*`）同样要写内核的 `node_modules`。
# 所以「随包内核只读」在非 root 用户下等于「直接起不来」。
#
# 应用自己无权 chown（也不该为此引入提权面，见 src-tauri/src/service/perm.rs 的
# 说明），因此由本脚本（dpkg 以 root 运行）把随包资源树的属主交给真正使用桌面端的
# 用户。升级/重装时本脚本会再跑一遍 —— dpkg 解包会把新文件重新落成 root 所有，
# 属主不会被漏掉。
#
# 约定：
#   * 只在 configure 阶段做事；
#   * 任何失败都不让安装失败（postinst 返回非 0 会让 dpkg 报「配置失败」）：确定不了
#     用户时只打印可粘贴的 chown 指引，chown 失败也只告警；
#   * 普通（非离线）安装的资源树里没有 `dsh/`，本脚本直接退出。
#
# 测试开关：`DSH_DESKTOP_RESOURCES_DIR` 覆盖资源根，`DRY_RUN=1` 只打印不执行。

set -u

RESOURCES_DIR="${DSH_DESKTOP_RESOURCES_DIR:-/usr/lib/Deepseek Harness Desktop/resources}"
DRY_RUN="${DRY_RUN:-0}"

log() {
	printf 'deepseek-harness-desktop: %s\n' "$*" >&2
}

# 只处理 configure；abort-upgrade / abort-remove 等阶段不做任何事。
[ "${1:-configure}" = "configure" ] || exit 0

# 离线包才需要接管属主：普通安装没有随包内核。
[ -d "${RESOURCES_DIR}/dsh" ] || exit 0

# uid → 用户名。
user_from_uid() {
	[ -n "${1:-}" ] || return 1
	getent passwd "$1" 2>/dev/null | cut -d: -f1 | head -n 1
}

# 兜底：唯一一个装过本应用数据目录（`~/.local/share/dsh-tauri` 或 `~/.dsh`）的普通
# 用户。多数用户，多个命中说明这台机器上分不清是谁在用 —— 那时宁可不猜。
user_from_appdata() {
	found=""
	count=0
	while IFS=: read -r name _passwd uid _gid _gecos home _shell; do
		case "$uid" in '' | *[!0-9]*) continue ;; esac
		[ "$uid" -ge 1000 ] || continue
		[ -n "$home" ] && [ -d "$home" ] || continue
		[ -e "$home/.local/share/dsh-tauri" ] || [ -e "$home/.dsh" ] || continue
		found="$name"
		count=$((count + 1))
	done <<EOF
$(getent passwd 2>/dev/null)
EOF
	[ "$count" -eq 1 ] && printf '%s\n' "$found"
	return 0
}

# 1) 安装者身份：sudo 装 → SUDO_USER；doas → DOAS_USER；GUI/图形安装器 → PKEXEC_UID。
candidates=""
for candidate in "${SUDO_USER:-}" "${DOAS_USER:-}"; do
	[ -n "$candidate" ] && candidates="${candidates} ${candidate}"
done
if [ -n "${PKEXEC_UID:-}" ]; then
	pkexec_user="$(user_from_uid "${PKEXEC_UID}" || true)"
	[ -n "$pkexec_user" ] && candidates="${candidates} ${pkexec_user}"
fi

user=""
for candidate in $candidates; do
	[ "$candidate" = "root" ] && continue
	if id -u "$candidate" >/dev/null 2>&1; then
		user="$candidate"
		break
	fi
done

# 2) 兜底：按应用数据目录反推。
[ -n "$user" ] || user="$(user_from_appdata || true)"

if [ -z "$user" ]; then
	log "cannot tell which user runs the desktop app, but the bundled dsh core must be writable by it:"
	log "  sudo chown -R <your-user>:<your-group> \"${RESOURCES_DIR}\""
	exit 0
fi

group="$(id -gn "$user" 2>/dev/null || printf '%s' "$user")"

if [ "${DRY_RUN}" = "1" ]; then
	log "dry run: chown -R ${user}:${group} ${RESOURCES_DIR}"
	exit 0
fi

# 整棵随包资源树（dsh 内核、内置插件、pnpm、node）都交给该用户：内核根、内置插件目录
# 与 JS 补丁都在这里面，逐个列举只会漏。同机其他用户仍不可写 —— 属主只有一个。
if chown -R "${user}:${group}" "${RESOURCES_DIR}" 2>/dev/null; then
	log "bundled runtimes under ${RESOURCES_DIR} are now writable by ${user}"
else
	log "warning: chown -R ${user}:${group} \"${RESOURCES_DIR}\" failed"
	log "  the bundled dsh core must be writable by ${user}; run that command manually as root"
fi

exit 0
