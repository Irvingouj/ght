# zellij driver: a detached zellij session inside a headless conhost, so the
# pane gets a real console without opening a window.
WORKM="$(cygpath -m "$WORK")"
ZJ() { MSYS_NO_PATHCONV=1 env -u ZELLIJ -u ZELLIJ_SESSION_NAME -u ZELLIJ_PANE_ID zellij --config "$WORKM/zellij.kdl" "$@"; }
Z() { ZJ --session ght-e2e "$@"; }
K_ENTER=Enter K_ESC=Esc K_TAB=Tab K_CD=Ctrl_d K_CU=Ctrl_u K_CE=Ctrl_e K_CY=Ctrl_y K_CP=Ctrl_p K_CW=Ctrl_w K_DOWN=Down K_SPACE=Space K_BS=Backspace
start() {
  printf 'show_startup_tips false\nshow_release_notes false\npane_frames false\n' > "$WORK/zellij.kdl"
  printf 'layout {\n    pane command="cmd.exe" { args "/c" "%s"; }\n}\n' "$WORKM/launch.cmd" > "$WORK/layout.kdl"
  stop
  local zellij; zellij="$(cygpath -w "$(command -v zellij)")"
  env -u ZELLIJ -u ZELLIJ_SESSION_NAME -u ZELLIJ_PANE_ID powershell.exe -NoProfile -Command \
    "Start-Process conhost.exe -ArgumentList @('--headless','--width','$1','--height','$2','$zellij','--config','$WORKM/zellij.kdl','-n','$WORKM/layout.kdl','-s','ght-e2e') -WindowStyle Hidden"
  sleep 2
}
stop() { ZJ kill-session ght-e2e >/dev/null 2>&1; ZJ delete-session ght-e2e >/dev/null 2>&1; }
c() { Z action write-chars "$1"; sleep "${2:-0.5}"; }
k() { Z action send-keys "${1//_/ }"; sleep 0.5; }
d() { Z action dump-screen --path "$WORKM/screen.txt"; sed -e 's/[[:space:]]*$//' "$WORK/screen.txt"; }
cur() { Z action dump-screen --ansi --path "$WORKM/screen.ansi"; python -I "$HERE/cursor_row.py" "$WORK/screen.ansi" | cut -c1-160; }
