#!/usr/bin/env python3
"""Butters detection lab: recreate a heavy agent-user's machine, then score
what Butters picks up.

    python3 scripts/lab/lab.py up      # start ~45 scenarios in ~/.butters-lab
    python3 scripts/lab/lab.py down    # stop everything the lab started

Every process runs under ~/.butters-lab (never your real projects) and copies
the *real* process shape of the tool it stands in for: the same executable
name and path layout, the same argv, wrapper chains (npm -> sh -> vite), a
listening port where the real tool has one, and an "orphan" (terminal gone)
or "inside a terminal" parent. The verdicts are scored by `cargo run
--example lab` in src-tauri, which runs the real Butters engine.
"""
import json
import os
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path

LAB = Path.home() / ".butters-lab"
STATE = LAB / "state.json"
HOG = LAB / "bin" / "hog"
NODE = shutil.which("node") or "/opt/homebrew/bin/node"
PY = shutil.which("python3") or "/opt/homebrew/bin/python3"
BUN = shutil.which("bun")
DENO = shutil.which("deno")

# Small JS bodies. SERVER listens on a port (like every dev server), IDLE
# just holds memory, SPIN burns a core, LEAK grows.
JS = {
    "server": "const b=Buffer.alloc((+process.env.MB||60)*1048576,1);process.getBuiltinModule('http').createServer((q,s)=>s.end('ok')).listen(+process.env.PORT||0,'127.0.0.1');setInterval(()=>b[0]++,1e9)",
    "idle": "const b=Buffer.alloc((+process.env.MB||80)*1048576,1);setInterval(()=>b[0]++,1e9)",
    "spin": "for(;;){Math.sqrt(Math.random())}",
}
PYSRV = "import http.server,os,socketserver;b=bytearray(int(os.environ.get('MB','60'))*1048576);socketserver.TCPServer(('127.0.0.1',int(os.environ.get('PORT','0'))),http.server.SimpleHTTPRequestHandler).serve_forever()"
PYIDLE = "import os,time;b=bytearray(int(os.environ.get('MB','80'))*1048576)\nwhile True: time.sleep(5)"

# id, what a person calls it, expectation, how to start it.
#   expect: "suggest"  -> must be listed (recommended or not)
#           "pretick"  -> must be listed AND pre-ticked
#           "quiet"    -> must NOT be listed (fresh / active / protected work)
#           "protect"  -> must be protected, never closed
# "orphan": parent exits right away (terminal closed). "aged": the lab
# scorer ages it (idle 3 h) so idle rules can fire without waiting hours.
SCENARIOS = []


def S(id, who, expect, kind, *, orphan=True, aged=True, **kw):
    SCENARIOS.append(dict(id=id, who=who, expect=expect, kind=kind, orphan=orphan, aged=aged, **kw))


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
S("flutter", "flutter run (dart tool)", "pretick", "native", exe="flutter_tools.snapshot", name="dart", argv=["dart", "--packages=x", "flutter_tools.snapshot", "run"], mb=400)
S("android-emu", "Android emulator (qemu)", "suggest", "native", exe="emulator/qemu/darwin-aarch64/qemu-system-aarch64", name="qemu-system-aarch64", mb=900, orphan=False)
S("gradle", "Gradle daemon (Android build)", "suggest", "native", exe="gradle/GradleDaemon", name="java", argv=["java", "-Xmx2g", "-cp", "gradle-launcher.jar", "org.gradle.launcher.daemon.bootstrap.GradleDaemon", "8.10"], mb=700, orphan=True)
# ---- python / ruby / go / rust / php ----
S("uvicorn", "FastAPI via uvicorn", "pretick", "py", mod="uvicorn", args=["main:app", "--reload"], port=8001)
S("django", "Django runserver", "pretick", "py-script", script="manage.py", args=["runserver"], port=8002)
S("flask", "flask run", "pretick", "py", mod="flask", args=["run", "--port", "5050"], port=5050)
S("streamlit", "Streamlit", "pretick", "py", mod="streamlit", args=["run", "app.py"], port=8501)
S("jupyter", "Jupyter Lab", "pretick", "py", mod="jupyterlab", port=8888)
S("py-http", "python -m http.server", "pretick", "py", mod="http.server", port=8003)
S("rails", "Rails server (puma)", "pretick", "native", exe="bin/puma", name="ruby", argv=["ruby", "bin/rails", "server"], port=3004)
S("go-air", "Go air live reload", "pretick", "native", exe="go/bin/air", name="air", port=8004)
S("go-run", "go run ./cmd/api (built binary on a port)", "pretick", "native", exe="go-build/b001/exe/api", name="api", port=8005)
S("cargo-watch", "cargo watch -x run", "pretick", "native", exe=".cargo/bin/cargo-watch", name="cargo-watch", argv=["cargo-watch", "-x", "run"])
S("php", "php -S localhost", "pretick", "native", exe="php", name="php", argv=["php", "-S", "127.0.0.1:8006"], port=8006)
# ---- coding agents (idle a few hours, terminal still open) ----
S("claude", "Claude Code session", "pretick", "native", exe=".local/share/claude/versions/2.1.288", name="2.1.288", argv=["claude"], mb=300, orphan=True)
S("claude-in-term", "Claude Code in an open terminal", "suggest", "native", exe=".local/share/claude/versions/2.1.288", name="2.1.288", argv=["claude", "--resume"], mb=300, orphan=False)
S("codex", "Codex CLI (node)", "pretick", "node", pkg="@openai/codex", bin="bin/codex.js", mb=200)
S("codex-native", "Codex native binary", "pretick", "native", exe="@openai/codex/vendor/aarch64-apple-darwin/codex/codex", name="codex", mb=200)
S("gemini", "Gemini CLI", "pretick", "node", pkg="@google/gemini-cli", bin="dist/index.js", mb=200)
S("opencode", "OpenCode", "pretick", "native", exe=".opencode/bin/opencode", name="opencode", mb=200)
S("aider", "Aider (python)", "pretick", "native", exe=".local/bin/aider", name="python3", argv=["python3", "/Users/x/.local/bin/aider"], mb=200)
S("goose", "Goose", "pretick", "native", exe=".local/bin/goose", name="goose", argv=["goose", "session"], mb=200)
S("ggcoder", "GG Coder CLI", "pretick", "node", pkg="@kenkaiiii/ggcoder", bin="dist/cli.js", mb=200)
S("cursor-agent", "Cursor Agent", "pretick", "native", exe=".local/bin/cursor-agent", name="cursor-agent", mb=200)
S("amp", "Amp", "pretick", "node", pkg="@sourcegraph/amp", bin="dist/main.js", mb=200)
S("copilot", "GitHub Copilot CLI", "pretick", "node", pkg="@github/copilot", bin="index.js", mb=200)
S("droid", "Factory Droid", "pretick", "native", exe=".factory/bin/droid", name="droid", mb=200)
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
S("leak", "leaking worker", "suggest", "node-script", script="worker.js", leak=True, orphan=False)
# ---- a busy day: many sessions of the same tools across many projects ----
for _i, _proj in enumerate(["shop-web", "admin", "landing", "mobile-app", "api", "docs", "blog", "dashboard"]):
    S(f"many-vite-{_i}", f"Vite in {_proj} (one of 8)", "pretick", "node", pkg="vite", bin="bin/vite.js", args=["--port", str(5200 + _i)], port=5200 + _i)
for _i in range(6):
    S(f"many-claude-{_i}", f"Claude Code session {_i + 1} of 6", "pretick", "native", exe=".local/share/claude/versions/2.1.288", name="2.1.288", argv=["claude"], mb=250)
for _i in range(4):
    S(f"many-mcp-{_i}", f"Playwright MCP {_i + 1} of 4 (one per session)", "pretick", "npx", pkg="@playwright/mcp", bin="cli.js", mb=60)

# ---- must stay alone ----
S("fresh-vite", "Vite you started a minute ago (in a terminal)", "quiet", "npm", pkg="vite", bin="bin/vite.js", port=5177, orphan=False, aged=False)
S("fresh-claude", "Claude Code you're using right now", "quiet", "native", exe=".local/share/claude/versions/2.1.288", name="2.1.288", argv=["claude"], mb=300, orphan=False, aged=False)
S("small-script", "tiny one-off script (no port, small)", "quiet", "node-script", script="tiny.js", js="idle", mb=5, orphan=False)
S("redis", "redis-server (brew service, launchd)", "quiet", "native", exe="redis-server", name="redis-server", port=6380, orphan=True)
S("postgres", "postgres (brew service, launchd)", "quiet", "native", exe="postgres", name="postgres", argv=["postgres", "-D", "/opt/homebrew/var/postgresql@17"], port=5433, orphan=True)


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


def write_pkg(proj, pkg, binpath, body):
    f = proj / "node_modules" / pkg / binpath
    f.parent.mkdir(parents=True, exist_ok=True)
    f.write_text(body)
    return f


def native_copy(rel, name):
    """A copy of the stand-in binary at a realistic path, so exe/argv match the real tool."""
    target = LAB / "native" / rel
    target.parent.mkdir(parents=True, exist_ok=True)
    if not target.exists():
        shutil.copy2(HOG, target)
    return target


def launch(sc):
    proj = project(sc["id"])
    env = {**os.environ, "MB": str(sc.get("mb", 60)), "PORT": str(sc.get("port", 0)), "HOG_MB": str(sc.get("mb", 60))}
    if sc.get("port") is not None:
        env["HOG_PORT"] = str(sc["port"])
    if sc.get("leak"):
        env["HOG_GROW"] = "8"
    js = JS[sc.get("js", "server" if sc.get("port") else "idle")]
    k = sc["kind"]
    if k in ("node", "npx"):
        f = write_pkg(proj, sc["pkg"], sc["bin"], js)
        argv = [NODE, str(f), *sc.get("args", [])]
        if k == "npx":
            # npx: `npm exec` (node) -> sh -c -> node .../cli.js
            argv = ["/bin/sh", "-c", f'exec -a "npm exec {sc["pkg"]}" {NODE} -e "require(\'child_process\').spawn(process.argv[1],process.argv.slice(2),{{stdio:\'inherit\'}})" {NODE} {f}']
    elif k == "npm":
        f = write_pkg(proj, sc["pkg"], sc["bin"], js)
        # npm run dev: node (titled "npm run dev") -> sh -c vite -> node vite.js
        argv = ["/bin/sh", "-c", f'exec -a "npm run dev" {NODE} -e "require(\'child_process\').spawn(\'/bin/sh\',[\'-c\',process.argv[1]],{{stdio:\'inherit\'}});setInterval(()=>{{}},1e9)" "{NODE} {f} {" ".join(sc.get("args", []))}"']
    elif k == "pnpm":
        f = write_pkg(proj, sc["pkg"], sc["bin"], js)
        pnpm_js = write_pkg(proj, ".pnpm-bin", "pnpm.cjs", f"require('child_process').spawn('/bin/sh',['-c','{NODE} {f}'],{{stdio:'inherit'}});setInterval(()=>{{}},1e9)")
        argv = [NODE, str(pnpm_js), "dev"]
    elif k == "next":
        f = write_pkg(proj, "next", "dist/bin/next", f"require('child_process').spawn(process.execPath,['-e',{json.dumps(JS['server'])}],{{stdio:'inherit',argv0:'next-server (v15.5.0)'}});setInterval(()=>{{}},1e9)")
        argv = [NODE, str(f), "dev"]
    elif k == "node-script":
        f = proj / sc["script"]
        f.write_text(JS[sc.get("js", "server" if sc.get("port") else "idle")] if not sc.get("leak") else "const k=[];setInterval(()=>k.push(Buffer.alloc(8*1048576,1)),2000)")
        argv = [NODE, str(f)]
    elif k == "bun":
        if not BUN:
            return None
        f = proj / sc["script"]
        f.write_text(f"const b=new Uint8Array(60*1048576).fill(1);Bun.serve({{port:{sc['port']},hostname:'127.0.0.1',fetch(){{return new Response('ok')}}}})")
        argv = [BUN, "--hot", str(f)]
    elif k == "deno":
        if not DENO:
            return None
        f = proj / sc["script"]
        f.write_text(f"const b=new Uint8Array(60*1048576).fill(1);Deno.serve({{port:{sc['port']},hostname:'127.0.0.1'}},()=>new Response('ok'))")
        argv = [DENO, "run", "-A", "--watch", str(f)]
    elif k == "py":
        mod = sc["mod"]
        pkg = proj / "site" / mod.replace(".", "/")
        pkg.mkdir(parents=True, exist_ok=True)
        (pkg / "__init__.py").write_text("")
        (pkg / "__main__.py").write_text(PYSRV if sc.get("port") else PYIDLE)
        env["PYTHONPATH"] = str(proj / "site")
        argv = [PY, "-m", mod, *sc.get("args", [])]
    elif k == "py-script":
        f = proj / sc["script"]
        f.write_text(PYSRV)
        argv = [PY, str(f), *sc.get("args", [])]
    elif k == "native":
        exe = native_copy(sc["exe"], sc["name"])
        shown = sc.get("argv") or [sc["name"]]
        if sc["name"] != Path(sc["exe"]).name:
            # macOS reports the process name from the file name; a hard link
            # with the shown name makes it match the real tool.
            alias = exe.parent / sc["name"]
            if not alias.exists():
                os.link(exe, alias)
            exe = alias
        # A tiny python shim execs the binary with argv[0] set like the real tool.
        argv = [PY, "-c", "import os,sys;os.execv(sys.argv[1],sys.argv[2:])", str(exe), *shown]
    else:
        raise ValueError(k)

    if sc["orphan"]:
        # Double fork: the intermediate exits at once, so the real process is
        # re-parented to launchd ("terminal closed"). Its own session, so it
        # outlives this script.
        r, w = os.pipe()
        if os.fork() == 0:
            os.setsid()
            pid = os.fork()
            if pid == 0:
                os.chdir(proj)
                devnull = os.open(os.devnull, os.O_RDWR)
                for fd in (0, 1, 2):
                    os.dup2(devnull, fd)
                os.execve(argv[0], argv, env)
            os.write(w, str(pid).encode())
            os._exit(0)
        os.close(w)
        pid = int(os.read(r, 32).decode())
        os.close(r)
        os.wait()
        return pid
    # In a "terminal": an interactive login zsh (like a terminal tab) stays
    # the parent and stays alive, with the tool running inside it.
    term = subprocess.Popen(["/bin/zsh", "-il"], env=env, cwd=proj, stdin=subprocess.PIPE,
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
    term.stdin.write((subprocess.list2cmdline(argv) + "\n").encode())
    term.stdin.flush()
    return term.pid


def up():
    assert HOG.exists(), "build it first: rustc -O -o ~/.butters-lab/bin/hog scripts/lab/hog.rs"
    down(quiet=True)
    st = {"procs": {}, "started": time.time()}
    for sc in SCENARIOS:
        try:
            pid = launch(sc)
        except Exception as e:
            print(f"  ! {sc['id']}: {e}")
            continue
        if pid:
            st["procs"][sc["id"]] = pid
    save(st)
    (LAB / "scenarios.json").write_text(json.dumps(SCENARIOS, indent=1))
    print(f"started {len(st['procs'])} of {len(SCENARIOS)} scenarios in {LAB}")


def down(quiet=False):
    # Everything whose command line points into the lab folder, plus the
    # recorded roots. Only the lab: nothing else matches.
    out = subprocess.run(["ps", "-axo", "pid=,command="], capture_output=True, text=True).stdout
    me = os.getpid()
    n = 0
    for line in out.splitlines():
        pid_s, _, cmd = line.strip().partition(" ")
        if str(LAB) in cmd and int(pid_s) != me and "lab.py" not in cmd:
            try:
                os.kill(int(pid_s), signal.SIGKILL)
                n += 1
            except ProcessLookupError:
                pass
    for pid in state().get("procs", {}).values():
        for kill in (lambda p: os.killpg(p, signal.SIGKILL), lambda p: os.kill(p, signal.SIGKILL)):
            try:
                kill(pid)
            except (ProcessLookupError, PermissionError):
                pass
    # Anything whose working folder is inside the lab (python -m ..., native
    # stand-ins whose argv doesn't mention the lab).
    lsof = subprocess.run(["lsof", "-a", "-d", "cwd", "-u", str(os.getuid()), "-Fpn"], capture_output=True, text=True).stdout
    pid = None
    for line in lsof.splitlines():
        if line.startswith("p"):
            pid = int(line[1:])
        elif line.startswith("n") and pid and line[1:].startswith(str(LAB)) and pid != me:
            try:
                os.kill(pid, signal.SIGKILL)
                n += 1
            except ProcessLookupError:
                pass
    if STATE.exists():
        STATE.unlink()
    if not quiet:
        print(f"stopped {n} lab processes")


if __name__ == "__main__":
    {"up": up, "down": down}[sys.argv[1] if len(sys.argv) > 1 else "up"]()
