"""Print the reverse-video runs of an ANSI screen dump: the cursor row, plus badges."""
import re
import sys

sys.stdout.reconfigure(encoding="utf-8")
data = open(sys.argv[1], encoding="utf-8", errors="replace").read()
reverse = False
runs, run = [], []
for m in re.finditer(r"\x1b\[([0-9;]*)m|([^\x1b])", data):
    if m.group(1) is not None:
        for p in (m.group(1) or "0").split(";"):
            if p == "0" or p == "27":
                reverse = False
            elif p == "7":
                reverse = True
    else:
        ch = m.group(2)
        if reverse and ch != "\n":
            run.append(ch)
        elif run:
            runs.append("".join(run))
            run = []
if run:
    runs.append("".join(run))
for r in runs:
    if r.strip():
        print(r.rstrip())
