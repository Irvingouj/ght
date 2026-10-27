# tmux driver: ght runs in a detached tmux session inside WSL; the Windows
# binary starts through WSL interop.
# -e runs tmux without a shell in between, so arguments arrive unchanged.
T() { MSYS_NO_PATHCONV=1 wsl.exe -d Ubuntu -e tmux "$@"; }
K_ENTER=Enter K_ESC=Escape K_TAB=Tab K_CD=C-d K_CU=C-u K_CE=C-e K_CY=C-y K_CP=C-p K_CW=C-w K_DOWN=Down K_SPACE=Space K_BS=BSpace
start() {
  T kill-session -t ght-e2e 2>/dev/null
  T new-session -d -s ght-e2e -x "$1" -y "$2" "cmd.exe /c '$(win "$WORK/launch.cmd")'; sleep 600"
}
stop() { T kill-session -t ght-e2e 2>/dev/null; }
c() { T send-keys -t ght-e2e -l -- "$1"; sleep "${2:-0.5}"; }
k() { T send-keys -t ght-e2e "$1"; sleep 0.5; }
d() { T capture-pane -t ght-e2e -p | sed -e 's/[[:space:]]*$//'; }
cur() { T capture-pane -t ght-e2e -p -e > "$WORK/screen.ansi"; python -I "$HERE/cursor_row.py" "$WORK/screen.ansi" | cut -c1-160; }
