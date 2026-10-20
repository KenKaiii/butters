#!/usr/bin/env python3
"""Butters detection lab: recreate a heavy agent-user's machine, then score
what Butters picks up. Works on macOS and Windows (and runs in CI on both).

    python scripts/lab/lab.py up      # start ~76 scenarios in ~/.butters-lab
    cd src-tauri && cargo run --release --example lab   # score them (exit 1 on any miss)
    python scripts/lab/lab.py down    # stop everything the lab started

Every process runs under ~/.butters-lab (never your real projects) and copies
the *real* process shape of the tool it stands in for, per platform: the same
executable name and folder layout (claude / claude.exe, the Android SDK's
qemu, Go's go-build temp folder...), the same arguments, the same wrapper
chains (npm -> sh -c / cmd /c -> vite), a listening port where the real tool
has one, and either an "orphan" parent (the terminal that started it is gone)
or a terminal shell that is still open. Native tools are played by hog.rs,
copied to the real tool's path and name.
"""
import json
import os
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path

WIN = os.name == "nt"
EXE = ".exe" if WIN else ""
HERE = Path(__file__).resolve().parent
LAB = Path.home() / ".butters-lab"
STATE = LAB / "state.json"
HOG = LAB / "bin" / f"hog{EXE}"
NODE = shutil.which("node")
PY = sys.executable
BUN = shutil.which("bun")
DENO = shutil.which("deno")

# Small JS bodies. SERVER listens on a port (like every dev server), IDLE just
# holds memory, SPIN burns a core, LEAK grows.
JS = {
    "server": "const b=Buffer.alloc((+process.env.MB||60)*1048576,1);process.getBuiltinModule('http').createServer((q,s)=>s.end('ok')).listen(+process.env.PORT||0,'127.0.0.1');setInterval(()=>b[0]++,1e9)",
    "idle": "const b=Buffer.alloc((+process.env.MB||80)*1048576,1);setInterval(()=>b[0]++,1e9)",
    "spin": "for(;;){Math.sqrt(Math.random())}",
    "leak": "const k=[];setInterval(()=>k.push(Buffer.alloc(8*1048576,1)),2000)",
}
PYSRV = "import http.server,os,socketserver;b=bytearray(int(os.environ.get('MB','60'))*1048576);socketserver.TCPServer(('127.0.0.1',int(os.environ.get('PORT','0'))),http.server.SimpleHTTPRequestHandler).serve_forever()"
PYIDLE = "import os,time;b=bytearray(int(os.environ.get('MB','80'))*1048576)\nwhile True: time.sleep(5)"

# A wrapper (npm, npx, pnpm) is a node script that renames itself like the
# real one and starts the real work through the platform shell, then stays.
WRAPPER = """
process.title = %(title)s;
const {spawn} = require('child_process');
const win = process.platform === 'win32';
spawn(win ? 'cmd.exe' : '/bin/sh', win ? ['/d', '/s', '/c', %(cmd)s] : ['-c', %(cmd)s], {stdio: 'ignore', windowsVerbatimArguments: win});
setInterval(() => {}, 1e9);
"""

# expect: "pretick" must be listed AND pre-ticked; "suggest" must be listed;
#         "quiet" must NOT be listed; "protect" must be protected.
# orphan: parent exits at once (terminal closed). aged: the scorer sets idle
# to 3 h (stands in for "left from yesterday").
SCENARIOS = []


def S(id, who, expect, kind, *, orphan=True, aged=True, **kw):
    SCENARIOS.append(dict(id=id, who=who, expect=expect, kind=kind, orphan=orphan, aged=aged, **kw))


def N(mac, win):
    """Native tool path inside the lab, per platform (Windows gets .exe)."""
    return win if WIN else mac


# ---- web dev servers (left behind after the terminal/agent closed) ----
S("vite", "Vite via npm run dev", "pretick", "npm", pkg="vite", bin="bin/vite.js", args=["--port", "5173"], port=5173)
S("vite-pnpm", "Vite via pnpm dev", "pretick", "pnpm", pkg="vite", bin="bin/vite.js", port=5174)
S("next", "Next.js dev (next-server child)", "pretick", "next", port=3000)
S("nuxt", "Nuxt dev", "pretick", "node", pkg="nuxi", bin="bin/nuxi.mjs", args=["dev"], port=3001)
S("astro", "Astro dev", "pretick", "node", pkg="astro", bin="astro.js", args=["dev"], port=4321)
S("remix", "React Router / Remix dev", "pretick", "node", pkg="@react-router/dev", bin="bin.js", args=["dev"], port=5175)
S("sveltekit", "SvelteKit (vite dev)", "pretick", "node", pkg="vite", bin="bin/vite.js", args=["dev"], port=5176)
S("storybook", "Storybook", "pretick", "node", pkg="storybook", bin="bin/index.cjs", args=["dev", "-p", "6006"], port=6006)
S("webpack", "webpack-dev-server", "pretick", "node", pkg="webpack-dev-server", bin="bin/webpack-dev-server.js", port=8080)
S("wrangler", "Cloudflare wrangler dev", "pretick", "node", pkg="wrangler", bin="bin/wrangler.js", args=["dev"], port=8787)
S("convex", "Convex dev", "pretick", "node", pkg="convex", bin="bin/main.js", args=["dev"])
S("bun-dev", "bun --hot server", "pretick", "bun", script="server.ts", port=3002)
S("deno-dev", "deno task dev", "pretick", "deno", script="main.ts", port=8000)
S("http-server", "npx http-server", "pretick", "node", pkg="http-server", bin="bin/http-server", port=8081)
S("plain-node", "node server.js on a port", "pretick", "node-script", script="server.js", port=3003)
# ---- mobile ----
S("expo", "Expo start (Metro)", "pretick", "node", pkg="expo", bin="bin/cli", args=["start"], port=8082)
S("rn-metro", "React Native Metro bundler", "pretick", "node", pkg="react-native", bin="cli.js", args=["start"], port=8083)
S("flutter", "flutter run (dart tool)", "pretick", "native", exe=N("flutter/bin/cache/dart-sdk/bin/dart", "flutter/bin/cache/dart-sdk/bin/dart.exe"),
  argv=["dart", "--packages=x", "{native}/flutter/bin/cache/flutter_tools.snapshot", "run"], mb=400)
S("android-emu", "Android emulator (qemu)", "suggest", "native", orphan=False, mb=900,
  exe=N("Library/Android/sdk/emulator/qemu/darwin-aarch64/qemu-system-aarch64", "AppData/Local/Android/Sdk/emulator/qemu/windows-x86_64/qemu-system-x86_64.exe"),
  argv=["qemu-system", "-avd", "Pixel_8"])
S("gradle", "Gradle daemon (Android build)", "suggest", "native", mb=700, exe=N("jdk/bin/java", "jdk/bin/java.exe"),
  argv=["java", "-Xmx2g", "-cp", "gradle-launcher.jar", "org.gradle.launcher.daemon.bootstrap.GradleDaemon", "8.10"])
# ---- python / ruby / go / rust / php ----
S("uvicorn", "FastAPI via uvicorn", "pretick", "py", mod="uvicorn", args=["main:app", "--reload"], port=8001)
S("django", "Django runserver", "pretick", "py-script", script="manage.py", args=["runserver"], port=8002)
S("flask", "flask run", "pretick", "py", mod="flask", args=["run", "--port", "5050"], port=5050)
S("streamlit", "Streamlit", "pretick", "py", mod="streamlit", args=["run", "app.py"], port=8501)
S("jupyter", "Jupyter Lab", "pretick", "py", mod="jupyterlab", port=8888)
S("py-http", "python -m http.server", "pretick", "py", mod="http.server", args=["8003", "--bind", "127.0.0.1"], port=8003, real=True)
S("rails", "Rails server (puma)", "pretick", "native", exe=N("ruby/bin/ruby", "ruby/bin/ruby.exe"), argv=["ruby", "bin/rails", "server"], port=3004)
S("go-air", "Go air live reload", "pretick", "native", exe=N("go/bin/air", "go/bin/air.exe"), argv=["air"], port=8004)
S("go-run", "go run ./cmd/api (built binary on a port)", "pretick", "native",
  exe=N("var/folders/T/go-build2731/b001/exe/api", "AppData/Local/Temp/go-build2731/b001/exe/api.exe"), argv=["api"], port=8005)
S("cargo-watch", "cargo watch -x run", "pretick", "native", exe=N(".cargo/bin/cargo-watch", ".cargo/bin/cargo-watch.exe"), argv=["cargo-watch", "-x", "run"])
S("php", "php -S localhost", "pretick", "native", exe=N("php/bin/php", "php/php.exe"), argv=["php", "-S", "127.0.0.1:8006"], port=8006)
# ---- coding agents ----
CLAUDE = N(".local/share/claude/versions/2.1.288", ".local/bin/claude.exe")
S("claude", "Claude Code session", "pretick", "native", exe=CLAUDE, argv=["claude"], mb=300)
S("claude-in-term", "Claude Code in an open terminal", "suggest", "native", exe=CLAUDE, argv=["claude", "--resume"], mb=300, orphan=False)
S("codex", "Codex CLI (node)", "pretick", "node", pkg="@openai/codex", bin="bin/codex.js", mb=200)
S("codex-native", "Codex native binary", "pretick", "native", mb=200,
  exe=N("@openai/codex/vendor/aarch64-apple-darwin/codex/codex", "@openai/codex/vendor/x86_64-pc-windows-msvc/codex/codex.exe"), argv=["codex"])
S("gemini", "Gemini CLI", "pretick", "node", pkg="@google/gemini-cli", bin="dist/index.js", mb=200)
S("opencode", "OpenCode", "pretick", "native", exe=N(".opencode/bin/opencode", ".opencode/bin/opencode.exe"), argv=["opencode"], mb=200)
S("aider", "Aider (python)", "pretick", "native", exe=N("py/bin/python3", "py/python.exe"), argv=["python3", "{native}/.local/bin/aider"], mb=200)
S("goose", "Goose", "pretick", "native", exe=N(".local/bin/goose", ".local/bin/goose.exe"), argv=["goose", "session"], mb=200)
S("ggcoder", "GG Coder CLI", "pretick", "node", pkg="@kenkaiiii/ggcoder", bin="dist/cli.js", mb=200)
S("cursor-agent", "Cursor Agent", "pretick", "native", exe=N(".local/bin/cursor-agent", ".local/bin/cursor-agent.exe"), argv=["cursor-agent"], mb=200)
S("amp", "Amp", "pretick", "node", pkg="@sourcegraph/amp", bin="dist/main.js", mb=200)
S("copilot", "GitHub Copilot CLI", "pretick", "node", pkg="@github/copilot", bin="index.js", mb=200)
S("droid", "Factory Droid", "pretick", "native", exe=N(".factory/bin/droid", ".factory/bin/droid.exe"), argv=["droid"], mb=200)
# ---- MCP servers (their agent session ended) ----
S("mcp-playwright", "Playwright MCP via npx", "pretick", "npx", pkg="@playwright/mcp", bin="cli.js", mb=60)
S("mcp-memory", "MCP memory server", "pretick", "node", pkg="@modelcontextprotocol/server-memory", bin="dist/index.js", mb=60)
S("mcp-py", "Python MCP via uvx", "pretick", "py", mod="mcp_server_fetch", mb=60)
S("mcp-context7", "Context7 MCP", "pretick", "node", pkg="@upstash/context7-mcp", bin="dist/index.js", mb=60)
# ---- watchers / builds / leaks / CPU ----
S("tsc-watch", "tsc --watch", "pretick", "node", pkg="typescript", bin="bin/tsc", args=["--watch"], mb=150)
S("vitest", "vitest watch", "pretick", "node", pkg="vitest", bin="vitest.mjs", mb=150)
S("tailwind", "tailwindcss --watch", "pretick", "node", pkg="tailwindcss", bin="lib/cli.js", args=["--watch"], mb=80)
S("busy", "runaway node build (100% CPU)", "suggest", "node-script", script="build.js", js="spin", orphan=False, aged=False)
S("leak", "leaking worker", "suggest", "node-script", script="worker.js", js="leak", orphan=False)
# ---- a busy day: many sessions of the same tools across many projects ----
for _i, _proj in enumerate(["shop-web", "admin", "landing", "mobile-app", "api", "docs", "blog", "dashboard"]):
    S(f"many-vite-{_i}", f"Vite in {_proj} (one of 8)", "pretick", "node", pkg="vite", bin="bin/vite.js", args=["--port", str(5200 + _i)], port=5200 + _i)
for _i in range(6):
    S(f"many-claude-{_i}", f"Claude Code session {_i + 1} of 6", "pretick", "native", exe=CLAUDE, argv=["claude"], mb=250)
for _i in range(4):
    S(f"many-mcp-{_i}", f"Playwright MCP {_i + 1} of 4 (one per session)", "pretick", "npx", pkg="@playwright/mcp", bin="cli.js", mb=60)
# ---- must stay alone ----
S("fresh-vite", "Vite you started a minute ago (in a terminal)", "quiet", "npm", pkg="vite", bin="bin/vite.js", port=5177, orphan=False, aged=False)
S("fresh-claude", "Claude Code you're using right now", "quiet", "native", exe=CLAUDE, argv=["claude"], mb=300, orphan=False, aged=False)
S("small-script", "tiny one-off script (no port, small)", "quiet", "node-script", script="tiny.js", js="idle", mb=5, orphan=False)
S("redis", "redis-server (a service)", "quiet", "native", exe=N("redis/bin/redis-server", "redis/redis-server.exe"), argv=["redis-server"], port=6380)
S("postgres", "postgres (a service)", "quiet", "native", exe=N("pg/bin/postgres", "pg/bin/postgres.exe"), argv=["postgres", "-D", "data"], port=5433)


# ------------------------------------------------------------------ helpers

def state():
    try:
        return json.loads(STATE.read_text())
    except (FileNotFoundError, json.JSONDecodeError):
        return {"procs": {}}


def save(st):
    STATE.write_text(json.dumps(st, indent=1))


def project(id):
    d = LAB / "projects" / id
    d.mkdir(parents=True, exist_ok=True)
    return d


def write(path, body):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(body)
    return path


def pkg_file(proj, pkg, binpath, body):
    return write(proj / "node_modules" / pkg / binpath, body)


def q(path):
    """A path quoted for a shell command line (paths with spaces on Windows)."""
    return f'"{path}"'


def build_hog():
    if HOG.exists():
        return
    HOG.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(["rustc", "-O", "-o", str(HOG), str(HERE / "hog.rs")], check=True)


def native_copy(rel):
    """The stand-in binary at the real tool's path and name."""
    target = LAB / "native" / rel
    target.parent.mkdir(parents=True, exist_ok=True)
    if not target.exists():
        shutil.copy2(HOG, target)
    return target


# ------------------------------------------------------------------ launching

def spawn_orphan(argv, exe, env, cwd):
    """Starts the tool with a parent that exits at once (the terminal is gone)."""
    if WIN:
        # A helper process starts it detached and exits: the tool's parent is gone.
        helper = (
            "import json,subprocess,sys;a=json.loads(sys.argv[1]);"
            "p=subprocess.Popen(a['argv'],executable=a['exe'],cwd=a['cwd'],env=a['env'],"
            "stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,"
            "creationflags=0x00000008|0x00000200);print(p.pid)"
        )
        out = subprocess.run([PY, "-c", helper, json.dumps({"argv": argv, "exe": exe, "cwd": str(cwd), "env": env})],
                             capture_output=True, text=True, check=True)
        return int(out.stdout.strip())
    # POSIX double fork: re-parented to launchd/init, its own session.
    r, w = os.pipe()
    if os.fork() == 0:
        os.setsid()
        pid = os.fork()
        if pid == 0:
            os.chdir(cwd)
            devnull = os.open(os.devnull, os.O_RDWR)
            for fd in (0, 1, 2):
                os.dup2(devnull, fd)
            os.execve(exe, argv, env)
        os.write(w, str(pid).encode())
        os._exit(0)
    os.close(w)
    pid = int(os.read(r, 32).decode())
    os.close(r)
    os.wait()
    return pid


def spawn_in_terminal(argv, exe, env, cwd):
    """Starts the tool inside a shell that stays open, like a terminal tab.
    Returns the shell's pid (the scenario's root)."""
    if WIN:
        line = subprocess.list2cmdline([exe, *argv[1:]])
        shell = subprocess.Popen(["cmd.exe", "/d", "/k", line], cwd=cwd, env=env, stdin=subprocess.PIPE,
                                 stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                                 creationflags=0x00000200 | 0x08000000)
        return shell.pid
    # zsh's ARGV0 sets the tool's argv[0] (e.g. "claude" for the versioned binary).
    line = f"ARGV0={q(argv[0])} {q(exe)} " + " ".join(q(a) for a in argv[1:])
    shell = subprocess.Popen(["/bin/zsh", "-il"], cwd=cwd, env=env, stdin=subprocess.PIPE,
                             stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
    shell.stdin.write((line + "\n").encode())
    shell.stdin.flush()
    return shell.pid


def launch(sc):
    proj = project(sc["id"])
    mb = sc.get("mb", 60)
    env = {**os.environ, "MB": str(mb), "PORT": str(sc.get("port", 0)), "HOG_MB": str(mb)}
    if sc.get("port") is not None:
        env["HOG_PORT"] = str(sc["port"])
    js = JS[sc.get("js", "server" if sc.get("port") else "idle")]
    k = sc["kind"]
    exe = NODE
    if k == "node":
        f = pkg_file(proj, sc["pkg"], sc["bin"], js)
        argv = ["node", str(f), *sc.get("args", [])]
    elif k in ("npm", "npx", "pnpm"):
        f = pkg_file(proj, sc["pkg"], sc["bin"], js)
        child = f"{q(NODE)} {q(f)} " + " ".join(sc.get("args", []))
        if WIN:
            # Like real npm on Windows: cmd /d /s /c "<command>". With /s, cmd
            # strips one outer pair of quotes, so the whole line gets its own.
            child = f'"{child.strip()}"'
        if k == "npm":
            title, wrapper, wargs = "npm run dev", pkg_file(proj, "npm", "bin/npm-cli.js", ""), ["run", "dev"]
        elif k == "npx":
            title, wrapper, wargs = f"npm exec {sc['pkg']}", pkg_file(proj, "npm", "bin/npx-cli.js", ""), [sc["pkg"]]
        else:
            title, wrapper, wargs = "pnpm dev", pkg_file(proj, "pnpm", "bin/pnpm.cjs", ""), ["dev"]
        wrapper.write_text(WRAPPER % {"title": json.dumps(title), "cmd": json.dumps(child)})
        argv = ["node", str(wrapper), *wargs]
    elif k == "next":
        server = json.dumps(JS["server"])
        f = pkg_file(proj, "next", "dist/bin/next",
                     f"require('child_process').spawn(process.execPath,['-e',{server}],{{stdio:'ignore',argv0:'next-server (v15.5.0)'}});setInterval(()=>{{}},1e9)")
        argv = ["node", str(f), "dev"]
    elif k == "node-script":
        f = write(proj / sc["script"], js)
        argv = ["node", str(f)]
    elif k == "bun":
        if not BUN:
            return None
        f = write(proj / sc["script"], f"const b=new Uint8Array(60*1048576).fill(1);Bun.serve({{port:{sc['port']},hostname:'127.0.0.1',fetch(){{return new Response('ok')}}}})")
        exe, argv = BUN, ["bun", "--hot", str(f)]
    elif k == "deno":
        if not DENO:
            return None
        f = write(proj / sc["script"], f"const b=new Uint8Array(60*1048576).fill(1);Deno.serve({{port:{sc['port']},hostname:'127.0.0.1'}},()=>new Response('ok'))")
        exe, argv = DENO, ["deno", "run", "-A", "--watch", str(f)]
    elif k == "py":
        mod = sc["mod"]
        if not sc.get("real"):
            # A stand-in package with the tool's module name, so `python -m
            # <tool>` looks exactly like the real thing. ("real" scenarios run
            # the genuine standard-library module instead.)
            site = proj / "site"
            write(site / mod.replace(".", "/") / "__init__.py", "")
            write(site / mod.replace(".", "/") / "__main__.py", PYSRV if sc.get("port") else PYIDLE)
            env["PYTHONPATH"] = str(site)
        exe, argv = PY, ["python", "-m", mod, *sc.get("args", [])]
    elif k == "py-script":
        f = write(proj / sc["script"], PYSRV)
        exe, argv = PY, ["python", str(f), *sc.get("args", [])]
    elif k == "native":
        exe = str(native_copy(sc["exe"]))
        native = str(LAB / "native")
        argv = [a.replace("{native}", native) for a in sc["argv"]]
    else:
        raise ValueError(k)
    if not exe:
        return None
    return (spawn_orphan if sc["orphan"] else spawn_in_terminal)(argv, exe, env, proj)


# ------------------------------------------------------------------ up / down

def up():
    if not NODE:
        sys.exit("Node.js is needed for the lab")
    build_hog()
    down(quiet=True)
    st = {"procs": {}, "failed": {}, "started": time.time()}
    for sc in SCENARIOS:
        try:
            pid = launch(sc)
        except Exception as e:  # report and keep going; the scorer counts it as a failure
            st["failed"][sc["id"]] = str(e)
            print(f"  ! {sc['id']}: {e}")
            continue
        if pid:
            st["procs"][sc["id"]] = pid
    save(st)
    (LAB / "scenarios.json").write_text(json.dumps(SCENARIOS, indent=1))
    ready = wait_until_ready(st)
    print(f"started {len(st['procs'])} of {len(SCENARIOS)} scenarios in {LAB} ({ready})")


def listening_ports():
    """Every TCP port something on this machine is listening on."""
    if WIN:
        out = subprocess.run(["netstat", "-ano", "-p", "TCP"], capture_output=True, text=True).stdout
        return {int(l.split()[1].rsplit(":", 1)[1]) for l in out.splitlines() if "LISTENING" in l}
    out = subprocess.run(["lsof", "-nP", "-iTCP", "-sTCP:LISTEN", "-Fn"], capture_output=True, text=True).stdout
    return {int(l.rsplit(":", 1)[1]) for l in out.splitlines() if l.startswith("n") and l.rsplit(":", 1)[1].isdigit()}


def lab_cmdlines():
    """Command lines of every process right now (to see which tools are up)."""
    if WIN:
        ps = ["powershell", "-NoProfile", "-Command", "(Get-CimInstance Win32_Process).CommandLine"]
        return subprocess.run(ps, capture_output=True, text=True).stdout
    return subprocess.run(["ps", "-axo", "command="], capture_output=True, text=True).stdout


def _is_up(scenario_id, cmds):
    """Is this scenario's tool running? `python -m <tool>` carries no folder
    in its command line, so it's recognised by its module instead."""
    sc = next(s for s in SCENARIOS if s["id"] == scenario_id)
    if sc["kind"] == "py":
        return f"-m {sc['mod']}" in cmds
    return f"projects/{scenario_id}/" in cmds


def wait_until_ready(st, timeout=float(os.environ.get("LAB_READY_TIMEOUT", "180"))):
    """Waits until every started scenario is really up, instead of a fixed
    sleep (slow CI machines need longer): servers listening on their port,
    and the tool's process running with the scenario's folder or name on its
    command line. Then a short extra settle so memory is filled in."""
    want_ports = {sc["port"] for sc in SCENARIOS if sc["id"] in st["procs"] and sc.get("port")}
    want_procs = {sc["id"] for sc in SCENARIOS if sc["id"] in st["procs"] and sc["kind"] != "native"}
    t0 = time.time()
    while True:
        missing_ports = want_ports - listening_ports()
        cmds = lab_cmdlines().replace("\\", "/")
        missing = {i for i in want_procs if not _is_up(i, cmds)}
        if not missing_ports and not missing:
            time.sleep(3)
            return f"all up after {time.time() - t0:.0f} s"
        if time.time() - t0 > timeout:
            return f"gave up after {timeout:.0f} s: ports {sorted(missing_ports)}, procs {sorted(missing)}"
        time.sleep(1)


def _kill(pid, tree=False):
    try:
        if WIN:
            subprocess.run(["taskkill", "/F", *(["/T"] if tree else []), "/PID", str(pid)], capture_output=True)
        elif tree:
            os.killpg(pid, signal.SIGKILL)
        else:
            os.kill(pid, signal.SIGKILL)
    except (ProcessLookupError, PermissionError, OSError):
        pass


def down(quiet=False):
    me = os.getpid()
    n = 0
    # 1. Every recorded scenario root, with its whole tree.
    for pid in state().get("procs", {}).values():
        _kill(pid, tree=True)
        _kill(pid)
        n += 1
    # 2. Anything else pointing into the lab (stragglers from an earlier run).
    lab = str(LAB)
    if WIN:
        ps = ["powershell", "-NoProfile", "-Command",
              "Get-CimInstance Win32_Process | Select-Object ProcessId,CommandLine,ExecutablePath | ConvertTo-Json -Compress"]
        out = subprocess.run(ps, capture_output=True, text=True).stdout or "[]"
        rows = json.loads(out)
        for r in rows if isinstance(rows, list) else [rows]:
            text = f"{r.get('CommandLine') or ''} {r.get('ExecutablePath') or ''}"
            if lab.lower() in text.lower() and r["ProcessId"] != me and "lab.py" not in text:
                _kill(r["ProcessId"])
                n += 1
    else:
        out = subprocess.run(["ps", "-axo", "pid=,command="], capture_output=True, text=True).stdout
        for line in out.splitlines():
            pid_s, _, cmd = line.strip().partition(" ")
            if lab in cmd and int(pid_s) != me and "lab.py" not in cmd:
                _kill(int(pid_s))
                n += 1
        # Processes whose working folder is in the lab (python -m ...).
        lsof = subprocess.run(["lsof", "-a", "-d", "cwd", "-u", str(os.getuid()), "-Fpn"], capture_output=True, text=True).stdout
        pid = None
        for line in lsof.splitlines():
            if line.startswith("p"):
                pid = int(line[1:])
            elif line.startswith("n") and pid and line[1:].startswith(lab) and pid != me:
                _kill(pid)
                n += 1
    if STATE.exists():
        STATE.unlink()
    if not quiet:
        print(f"stopped lab processes ({n} signalled)")


if __name__ == "__main__":
    {"up": up, "down": down}[sys.argv[1] if len(sys.argv) > 1 else "up"]()
