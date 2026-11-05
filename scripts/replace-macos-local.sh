#!/bin/bash
set -uo pipefail

keep_open=false
if [[ "${1:-}" == "--keep-open" ]]; then
  keep_open=true
  shift
fi

repo_dir="$(cd "$(/usr/bin/dirname "${BASH_SOURCE[0]}")/.." && pwd)" || exit 1
umask 077
log_dir="$repo_dir/.codex/logs"
log_file="$log_dir/package-and-replace-action.log"

run_action() {
  printf '\n[%s] 开始 BoltScribe 打包替换操作（PID %s）\n' "$(/bin/date '+%Y-%m-%d %H:%M:%S %z')" "$$"
  printf '操作日志：%s\n' "$log_file"
  cd "$repo_dir" || return 1
  # GUI actions can have a shorter PATH than the user's interactive shell.
  export PATH="${PATH:-/usr/bin:/bin:/usr/sbin:/sbin}:$HOME/.local/bin:/opt/homebrew/bin:/usr/local/bin:$HOME/.cargo/bin:/usr/bin:/bin:/usr/sbin:/sbin"
  local tool tool_bin
  for tool in node npm cargo; do
    tool_bin="$(command -v "$tool")" || {
      printf '错误：找不到 %s，请安装该工具或将其目录加入 PATH。\n' "$tool" >&2
      return 127
    }
    printf '使用 %s：%s\n' "$tool" "$tool_bin"
  done
  node "$repo_dir/scripts/replace-macos-local.mjs" "$@"
}

action_exit_code=0
if /bin/mkdir -p "$log_dir"; then
  run_action "$@" 2>&1 | /usr/bin/tee -a "$log_file"
  pipeline_codes=("${PIPESTATUS[@]}")
  action_exit_code="${pipeline_codes[0]}"
  if [[ "$action_exit_code" -eq 0 ]]; then
    action_exit_code="${pipeline_codes[1]}"
  fi
  printf '[%s] 操作结束，退出码 %s。\n' "$(/bin/date '+%Y-%m-%d %H:%M:%S %z')" "$action_exit_code" | /usr/bin/tee -a "$log_file"
else
  printf '无法创建操作日志目录：%s\n' "$log_dir" >&2
  action_exit_code=1
fi
if [[ "$action_exit_code" -ne 0 ]]; then
  printf '操作失败。详情：%s\n' "$log_file" >&2
fi
if [[ "$keep_open" == true && -t 0 && -t 1 ]]; then
  read -r -p '按回车结束此操作…' action_reply || true
fi
exit "$action_exit_code"
