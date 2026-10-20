//! Recognises what a process *is*: an agent session, a dev server, an MCP
//! server, a browser, a terminal... Pure string rules, no OS calls.

use super::model::{Kind, ProcInfo};

/// Lowercased file name without directories or a `.exe` suffix.
pub fn base(s: &str) -> String {
    let file = s.rsplit(['/', '\\']).next().unwrap_or(s);
    let lower = file.to_ascii_lowercase();
    lower.strip_suffix(".exe").map(str::to_string).unwrap_or(lower)
}

/// Splits on anything that is not a letter or digit, for whole-word matching
/// ("chrome-devtools-mcp" contains the word "mcp"; "mcpherson" does not).
fn words(s: &str) -> impl Iterator<Item = &str> {
    s.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty())
}

fn has_word(s: &str, word: &str) -> bool {
    words(s).any(|w| w.eq_ignore_ascii_case(word))
}

const RUNTIMES: &[&str] = &[
    "node", "bun", "deno", "python", "python3", "pythonw", "ruby", "php", "java", "dotnet", "go",
    "uv", "uvx", "npx", "bunx", "pnpx", "dart",
];

pub fn is_runtime(name: &str) -> bool {
    let n = base(name);
    RUNTIMES.contains(&n.as_str()) || (n.starts_with("python3.") && n.len() <= 10) || n.starts_with("python3.1")
}

const SHELLS: &[&str] = &[
    "zsh", "bash", "fish", "sh", "dash", "tcsh", "csh", "ksh", "nu", "xonsh", "elvish", "pwsh",
    "powershell", "cmd", "login",
];

/// Program name without the arguments some tools put in their process title
/// (npm renames itself to "npm run dev").
fn prog(p: &ProcInfo) -> String {
    let n = base(&p.name);
    n.split_whitespace().next().unwrap_or("").to_string()
}

pub fn is_shell(p: &ProcInfo) -> bool {
    let n = prog(p);
    let n = n.trim_start_matches('-'); // login shells show up as "-zsh"
    SHELLS.contains(&n)
}

/// A shell the user types into, as opposed to `sh -c "..."` used as glue.
pub fn is_interactive_shell(p: &ProcInfo) -> bool {
    is_shell(p)
        && !p
            .cmd
            .iter()
            .skip(1)
            .any(|a| a == "-c" || a.eq_ignore_ascii_case("/c") || a.eq_ignore_ascii_case("-command"))
}

/// Launchers that start the real work in a child (`npm run dev` -> `vite`).
/// Closing the launcher is what the user means when closing the dev server.
pub fn is_wrapper(p: &ProcInfo) -> bool {
    let n = prog(p);
    // npm runs as "node" but rewrites its argv to "npm run dev".
    let a0 = p.cmd.first().map(|a| base(a)).and_then(|a| a.split_whitespace().next().map(str::to_string)).unwrap_or_default();
    if [n.as_str(), a0.as_str()].iter().any(|x| matches!(
        *x,
        "npm" | "npx" | "pnpm" | "pnpx" | "yarn" | "bunx" | "uvx" | "turbo" | "nx" | "concurrently"
            | "nodemon" | "make" | "just" | "cargo" | "poetry" | "pipenv"
    )) {
        return true;
    }
    if is_shell(p) && !is_interactive_shell(p) {
        return true;
    }
    let args = arg_text(p, 3);
    match n.as_str() {
        "node" => ["npm-cli.js", "pnpm.cjs", "pnpm.mjs", "yarn.js", "yarn.cjs", "npx-cli.js", "concurrently", "nodemon", "turbo"]
            .iter()
            .any(|s| args.contains(s)),
        "bun" | "uv" => p.cmd.get(1).is_some_and(|a| a == "run" || a == "x"),
        _ => false,
    }
}

/// argv[0..n] lowercased and joined, for substring rules.
fn arg_text(p: &ProcInfo, n: usize) -> String {
    p.cmd.iter().take(n).map(|a| a.to_ascii_lowercase()).collect::<Vec<_>>().join(" ")
}

const AGENTS: &[(&str, &str)] = &[
    // (match, label). Matched against argv[0]'s file name, or package/script paths in argv.
    ("claude", "Claude Code"),
    ("@anthropic-ai/claude-code", "Claude Code"),
    ("codex", "Codex"),
    ("@openai/codex", "Codex"),
    ("ggcoder", "GG Coder"),
    ("@kenkaiiii/ggcoder", "GG Coder"),
    ("@kenkaiiii/gg-coder", "GG Coder"),
    ("gemini", "Gemini CLI"),
    ("@google/gemini-cli", "Gemini CLI"),
    ("aider", "Aider"),
    ("opencode", "OpenCode"),
    ("opencode-ai", "OpenCode"),
    ("cursor-agent", "Cursor Agent"),
    ("amp", "Amp"),
    ("@sourcegraph/amp", "Amp"),
    ("goose", "Goose"),
    ("qwen", "Qwen Code"),
    ("@qwen-code/qwen-code", "Qwen Code"),
    ("crush", "Crush"),
    ("copilot", "Copilot CLI"),
    ("@github/copilot", "Copilot CLI"),
    ("droid", "Droid"),
    ("pi-coding-agent", "Pi"),
    ("kimi", "Kimi CLI"),
    ("kiro-cli", "Kiro CLI"),
];

/// (needle, label, kind). Needles match whole argv tokens' file names or
/// substrings of the first few args, depending on the table.
const DEV_SERVERS: &[(&str, &str)] = &[
    ("vite", "Vite"),
    ("next-server", "Next.js"),
    ("next", "Next.js"),
    ("nuxt", "Nuxt"),
    ("nuxi", "Nuxt"),
    ("astro", "Astro"),
    ("remix", "Remix"),
    ("@remix-run/dev", "Remix"),
    ("react-router", "React Router"),
    ("@react-router/dev", "React Router"),
    ("react-scripts", "Create React App"),
    ("webpack-dev-server", "webpack"),
    ("webpack", "webpack"),
    ("parcel", "Parcel"),
    ("gatsby", "Gatsby"),
    ("storybook", "Storybook"),
    ("expo", "Expo"),
    ("metro", "Metro"),
    ("react-native", "React Native"),
    ("flutter_tools.snapshot", "Flutter"),
    ("flutter_tools", "Flutter"),
    ("wrangler", "Wrangler"),
    ("convex", "Convex"),
    ("vercel", "Vercel dev"),
    ("netlify", "Netlify dev"),
    ("firebase", "Firebase emulators"),
    ("uvicorn", "Uvicorn"),
    ("gunicorn", "Gunicorn"),
    ("hypercorn", "Hypercorn"),
    ("fastapi", "FastAPI"),
    ("flask", "Flask"),
    ("streamlit", "Streamlit"),
    ("gradio", "Gradio"),
    ("jupyter-lab", "Jupyter"),
    ("jupyterlab", "Jupyter"),
    ("jupyter-notebook", "Jupyter"),
    ("jupyter", "Jupyter"),
    ("manage", "Django"),
    ("rails", "Rails"),
    ("puma", "Puma"),
    ("hugo", "Hugo"),
    ("jekyll", "Jekyll"),
    ("http-server", "http-server"),
    ("live-server", "live-server"),
    ("serve", "serve"),
    ("tauri", "Tauri dev"),
    ("electron", "Electron dev"),
    ("air", "Air"),
    ("cargo-watch", "cargo-watch"),
    ("bacon", "bacon"),
    ("tsx", "tsx"),
    ("ts-node-dev", "ts-node-dev"),
];

const WATCHERS: &[(&str, &str)] = &[
    ("tsc", "TypeScript watch"),
    ("esbuild", "esbuild watch"),
    ("vitest", "Vitest"),
    ("jest", "Jest watch"),
    ("tailwindcss", "Tailwind watch"),
    ("chokidar", "chokidar"),
    ("watchman", "Watchman"),
    ("turbo", "Turborepo daemon"),
];

const LANGUAGE_SERVERS: &[(&str, &str)] = &[
    ("tsserver", "TypeScript server"),
    ("typescript-language-server", "TypeScript language server"),
    ("rust-analyzer", "rust-analyzer"),
    ("gopls", "gopls"),
    ("pyright", "Pyright"),
    ("pyright-langserver", "Pyright"),
    ("basedpyright", "basedpyright"),
    ("pylsp", "Python LSP"),
    ("ruff", "Ruff server"),
    ("eslint_d", "eslint_d"),
    ("prettierd", "prettierd"),
    ("biome", "Biome"),
    ("clangd", "clangd"),
    ("sourcekit-lsp", "SourceKit-LSP"),
    ("lua-language-server", "Lua language server"),
    ("tailwindcss-language-server", "Tailwind language server"),
    ("vscode-eslint-language-server", "ESLint language server"),
    ("svelteserver", "Svelte language server"),
    ("vue-language-server", "Vue language server"),
    ("jdtls", "Java language server"),
    ("kotlin-language-server", "Kotlin language server"),
    ("gradledaemon", "Gradle daemon"),
    ("kotlincompiledaemon", "Kotlin daemon"),
    ("org.gradle.launcher.daemon.bootstrap.gradledaemon", "Gradle daemon"),
    ("org.jetbrains.kotlin.daemon.kotlincompiledaemon", "Kotlin daemon"),
];

/// Phone emulators: heavy, and easy to forget running after a build.
const EMULATORS: &[(&str, &str)] = &[
    ("qemu-system-aarch64", "Android emulator"),
    ("qemu-system-x86_64", "Android emulator"),
    ("qemu-system-x86_64-headless", "Android emulator"),
    ("emulator", "Android emulator"),
];

const LOCAL_MODELS: &[(&str, &str)] = &[
    ("ollama", "Ollama"),
    ("llama-server", "llama.cpp server"),
    ("llama-cli", "llama.cpp"),
    ("mlx_lm.server", "MLX server"),
    ("vllm", "vLLM"),
    ("lms", "LM Studio"),
];

/// Tokens worth matching: argv[0] plus the next few args, reduced to file
/// names. For `node /x/node_modules/vite/bin/vite.js` that yields
/// ["node", "vite.js", ...] and we also keep the raw path for package rules.
fn candidate_tokens(p: &ProcInfo) -> Vec<String> {
    fn stem(a: &str) -> String {
        let b = base(a);
        let b = b.split_whitespace().next().unwrap_or("").to_string(); // "next-server (v15)" -> "next-server"
        b.trim_end_matches(".js")
            .trim_end_matches(".mjs")
            .trim_end_matches(".cjs")
            .trim_end_matches(".py")
            .to_string()
    }
    let mut out = vec![stem(&p.name)];
    // Flutter runs as `dart ... flutter_tools.snapshot run`: the snapshot is the tool.
    if prog(p) == "dart"
        && let Some(snap) = p.cmd.iter().find(|a| a.ends_with("flutter_tools.snapshot")) {
            out.push(base(snap));
        }
    let program = p.cmd.first().map(|a| stem(a)).unwrap_or_else(|| stem(&p.name));
    out.push(program.clone());
    // Only a runtime's *script* (or `python -m module`) names the real tool.
    // Later args are data (`--model gemini`) and must not change the verdict.
    // Java names its program by main class after the classpath options
    // (`java -Xmx2g -cp x.jar org.gradle...GradleDaemon`).
    if program == "java" {
        let mut it = p.cmd.iter().skip(1);
        while let Some(a) = it.next() {
            if a == "-cp" || a == "-classpath" || a == "-jar" {
                it.next();
                continue;
            }
            if !a.starts_with('-') {
                out.push(a.to_ascii_lowercase());
                break;
            }
        }
    }
    if is_runtime(&program) || is_runtime(&p.name) {
        let mut it = p.cmd.iter().skip(1);
        while let Some(a) = it.next() {
            if a == "-m" {
                if let Some(m) = it.next() {
                    out.push(stem(m));
                }
                break;
            }
            if a.starts_with('-') {
                continue;
            }
            out.push(stem(a));
            break;
        }
    }
    out
}

fn match_table(p: &ProcInfo, table: &[(&str, &str)]) -> Option<String> {
    let tokens = candidate_tokens(p);
    let paths = arg_text(p, 2);
    for (needle, label) in table {
        if needle.contains('/') || needle.contains('@') {
            if paths.contains(needle) {
                return Some((*label).to_string());
            }
        } else if tokens.iter().any(|t| t == needle) || package_dir_is(&paths, needle) {
            return Some((*label).to_string());
        }
    }
    None
}

/// True when a path in argv runs out of `node_modules/<pkg>/` (e.g. the real
/// script of `vite` is `.../node_modules/vite/bin/vite.js`).
fn package_dir_is(paths: &str, pkg: &str) -> bool {
    paths.contains(&format!("node_modules/{pkg}/")) || paths.contains(&format!("node_modules\\{pkg}\\"))
}

fn strip_ext(s: &str) -> String {
    s.trim_end_matches(".js").trim_end_matches(".mjs").trim_end_matches(".cjs").trim_end_matches(".py").to_string()
}

/// Name of the MCP server an argv token points at, if it points at one.
/// "@playwright/mcp@latest" -> "playwright", ".../chrome-devtools-mcp" ->
/// "chrome-devtools-mcp", ".../@modelcontextprotocol/server-memory/..." -> "memory".
fn mcp_name(arg: &str) -> Option<String> {
    let lower = arg.to_ascii_lowercase();
    let segs: Vec<&str> = lower.split(['/', '\\']).filter(|s| !s.is_empty()).collect();
    for (i, seg) in segs.iter().enumerate().rev() {
        if seg.starts_with("@modelcontextprotocol") {
            let next = segs.get(i + 1).map(|n| strip_ext(n.split('@').next().unwrap_or(n)));
            return Some(next.map(|n| n.trim_start_matches("server-").to_string()).unwrap_or_else(|| "modelcontextprotocol".into()));
        }
        let clean = seg.split('@').find(|s| !s.is_empty()).unwrap_or(seg);
        if has_word(clean, "mcp") || has_word(clean, "fastmcp") {
            // "@playwright/mcp" (as an npx arg or a node_modules path): the
            // scope is the useful name, not the generic word "mcp".
            if clean == "mcp" && i > 0 {
                let scope = segs[i - 1];
                if let Some(name) = scope.strip_prefix('@') {
                    return Some(name.to_string());
                }
            }
            if clean == "mcp" && seg.starts_with('@') {
                // "@playwright/mcp@latest" split as ["@playwright", "mcp@latest"]
                return Some(seg.trim_start_matches('@').to_string());
            }
            return Some(strip_ext(clean));
        }
    }
    None
}

fn is_mcp(p: &ProcInfo) -> Option<String> {
    // npm renames its process title to "npm exec @scope/pkg", so argv[0] can
    // hold several words; look at each of them as well as each argument.
    let words: Vec<&str> = p.cmd.iter().take(4).flat_map(|a| a.split_whitespace()).collect();
    for a in words {
        if a.starts_with('-') {
            continue;
        }
        if let Some(name) = mcp_name(a) {
            return Some(name);
        }
    }
    let n = base(&p.name);
    has_word(&n, "mcp").then_some(n)
}

/// Classifies a command-line process. Returns `None` for ordinary processes.
pub fn classify_cli(p: &ProcInfo) -> Option<(Kind, String)> {
    if let Some(label) = match_table(p, AGENTS) {
        // A bare runtime running some unrelated script named "amp.js" is unlikely,
        // but `claude mcp serve` is an MCP server, not a session.
        if p.cmd.iter().any(|a| a == "mcp") && p.cmd.iter().any(|a| a == "serve") {
            return Some((Kind::McpServer, format!("{label} MCP")));
        }
        return Some((Kind::AgentSession, label));
    }
    if let Some(name) = is_mcp(p) {
        return Some((Kind::McpServer, name));
    }
    if let Some(label) = match_table(p, LANGUAGE_SERVERS) {
        return Some((Kind::LanguageServer, label));
    }
    if let Some(label) = match_table(p, LOCAL_MODELS) {
        return Some((Kind::LocalModel, label));
    }
    // Only the Android SDK's emulator (its own folder), not any qemu.
    if let Some(label) = match_table(p, EMULATORS)
        && p.exe.as_deref().is_some_and(|e| e.contains("/emulator/") || e.to_ascii_lowercase().contains("android")) {
            return Some((Kind::LocalModel, label));
        }
    let watching = p.cmd.iter().any(|a| a == "--watch" || a == "-w" || a == "watch");
    if let Some(label) = match_table(p, WATCHERS) {
        let n = base(&p.name);
        let always = matches!(label.as_str(), "Watchman" | "Turborepo daemon" | "Vitest");
        if watching || always || n == "watchman" {
            return Some((Kind::Watcher, label));
        }
    }
    if let Some(label) = match_table(p, DEV_SERVERS) {
        // Generic words ("serve", "next", "expo"...) only count under a runtime
        // or as the program itself, and need a dev-ish verb for the ambiguous ones.
        let n = base(&p.name);
        let program_is_tool = candidate_tokens(p).first().is_some_and(|t| DEV_SERVERS.iter().any(|(k, _)| k == t));
        if is_runtime(&n) || program_is_tool || !p.ports.is_empty() {
            return Some((Kind::DevServer, label));
        }
    }
    // A program you built yourself, listening on a port: `go run`, `cargo run`,
    // `swift run`, `dotnet run`... (only from build folders, never installed apps).
    if !p.ports.is_empty()
        && let Some(exe) = p.exe.as_deref()
        && is_dev_build(exe) {
            return Some((Kind::DevServer, base(exe)));
        }
    // Something on a port under a runtime we don't know by name: call it by
    // its runtime ("Node dev server"); the row's detail shows the project
    // folder, which is how people tell their servers apart.
    if !p.ports.is_empty() && is_runtime(&p.name) {
        return Some((Kind::DevServer, runtime_label(&p.name)));
    }
    None
}

/// Root of the desktop app a process belongs to, if it belongs to one.
/// macOS: the outermost `.app` bundle. Windows: the program's exe path,
/// unless it is a runtime/shell (those are command-line work, not an app).
pub fn app_root(exe: &str) -> Option<String> {
    // Everything running inside the iOS Simulator is one thing, managed by Xcode.
    if exe.contains("/CoreSimulator/") {
        return Some(SIMULATOR_ROOT.to_string());
    }
    // Interpreters that macOS ships as an .app bundle (Homebrew/python.org
    // Python runs as ".../Python.app/Contents/MacOS/Python") are command-line
    // work: uvicorn, Django, Jupyter, Python MCP servers. Not one "Python" app.
    if is_bundled_interpreter(exe) {
        return None;
    }
    if let Some(i) = exe.find(".app/") {
        return Some(exe[..i + 4].to_string());
    }
    let windows_path = exe.contains('\\') || exe.as_bytes().get(1) == Some(&b':');
    if windows_path {
        let lower = exe.to_ascii_lowercase();
        let n = base(exe);
        if lower.starts_with("c:\\windows\\") || is_runtime(&n) || SHELLS.contains(&n.as_str()) {
            return None;
        }
        if matches!(n.as_str(), "conhost" | "openconsole" | "git" | "ssh" | "cargo" | "rustc" | "uv") {
            return None;
        }
        return Some(exe.to_string());
    }
    None
}

pub const SIMULATOR_ROOT: &str = "CoreSimulator";

/// "node" -> "Node", "python3.14" -> "Python", "Python" (Homebrew's bundle) -> "Python".
pub fn runtime_label(name: &str) -> String {
    let n = base(name);
    let n = n.split_whitespace().next().unwrap_or("");
    match n {
        "node" | "npx" => "Node",
        "bun" | "bunx" => "Bun",
        "deno" => "Deno",
        "ruby" => "Ruby",
        "php" => "PHP",
        "java" => "Java",
        "dotnet" => ".NET",
        "go" => "Go",
        "dart" => "Dart",
        "uv" | "uvx" => "Python",
        p if p.starts_with("python") => "Python",
        _ => "Local",
    }
    .to_string()
}

/// A binary that came out of a build, not an install: Go's build cache
/// (`go run`), Cargo/Swift/.NET/Zig build output, or `go build` into a
/// project. Never under /Applications, /System, /usr or Homebrew.
pub fn is_dev_build(exe: &str) -> bool {
    let e = exe.replace('\\', "/").to_ascii_lowercase();
    let installed = ["/applications/", "/system/", "/usr/", "/opt/homebrew/", "/library/", "c:/program files", "c:/windows/"];
    if installed.iter().any(|p| e.starts_with(p) || e.contains(".app/")) {
        return false;
    }
    let build_dirs = [
        "/go-build/",
        "/target/debug/",
        "/target/release/",
        "/.build/debug/",
        "/.build/release/",
        "/bin/debug/",
        "/bin/release/",
        "/zig-out/bin/",
        "/tmp/air",
        "/dist/",
        "/build/",
    ];
    build_dirs.iter().any(|d| e.contains(d))
}

fn is_bundled_interpreter(exe: &str) -> bool {
    let lower = exe.to_ascii_lowercase();
    (lower.contains("/python.framework/") || lower.contains("/resources/python.app/")) && lower.ends_with("/python")
}

/// Friendly name for an app root ("/Applications/Google Chrome.app" -> "Google Chrome").
pub fn app_name(root: &str) -> String {
    if root == SIMULATOR_ROOT {
        return "iOS Simulator apps".into();
    }
    let file = root.rsplit(['/', '\\']).next().unwrap_or(root);
    let stem = file.strip_suffix(".app").or_else(|| file.strip_suffix(".exe")).or_else(|| file.strip_suffix(".EXE")).unwrap_or(file);
    match stem.to_ascii_lowercase().as_str() {
        "chrome" => "Google Chrome".into(),
        "msedge" => "Microsoft Edge".into(),
        "brave" => "Brave".into(),
        "firefox" => "Firefox".into(),
        "code" => "VS Code".into(),
        "windowsterminal" => "Windows Terminal".into(),
        "docker desktop" => "Docker Desktop".into(),
        _ => stem.to_string(),
    }
}

pub fn classify_app(name: &str) -> Kind {
    let n = name.to_ascii_lowercase();
    const BROWSERS: &[&str] = &[
        "google chrome", "chrome", "chromium", "arc", "brave", "brave browser", "microsoft edge", "firefox",
        "safari", "dia", "comet", "zen", "zen browser", "vivaldi", "opera", "orion", "helium", "thorium",
    ];
    const TERMINALS: &[&str] = &[
        "terminal", "iterm", "iterm2", "ghostty", "wezterm", "wezterm-gui", "alacritty", "kitty", "warp",
        "hyper", "tabby", "wave", "windows terminal", "rio",
    ];
    const EDITORS: &[&str] = &[
        "vs code", "visual studio code", "code", "cursor", "windsurf", "zed", "intellij idea", "webstorm",
        "pycharm", "rustrover", "goland", "android studio", "xcode", "sublime text", "nova", "fleet",
        "trae", "kiro", "void", "positron",
    ];
    const CONTAINERS: &[&str] = &["docker", "docker desktop", "orbstack", "podman desktop", "rancher desktop"];
    const MODELS: &[&str] = &["ollama", "lm studio", "jan", "msty", "gpt4all"];
    let n = n.as_str();
    if BROWSERS.contains(&n) || n.starts_with("google chrome") {
        Kind::Browser
    } else if TERMINALS.contains(&n) {
        Kind::Terminal
    } else if EDITORS.contains(&n) || n.starts_with("visual studio code") {
        Kind::Editor
    } else if CONTAINERS.contains(&n) {
        Kind::ContainerRuntime
    } else if MODELS.contains(&n) {
        Kind::LocalModel
    } else {
        Kind::App
    }
}

/// Processes the OS needs. Never suggested, never closed, even if a rule
/// above would otherwise match. (Most are also owned by root/SYSTEM, which
/// already excludes them; this list covers the ones that run as the user.)
pub fn is_os_critical(p: &ProcInfo) -> bool {
    let n = base(&p.name);
    const NAMES: &[&str] = &[
        // macOS
        "kernel_task", "launchd", "windowserver", "loginwindow", "dock", "finder", "systemuiserver",
        "controlcenter", "notificationcenter", "coreservicesd", "cfprefsd", "distnoted", "usernoted",
        "universalaccessd", "secd", "trustd", "sharingd", "coreaudiod", "spotlight", "mds", "mds_stores",
        "mdworker", "mdworker_shared", "airportd", "bluetoothd", "powerd", "opendirectoryd", "securityd",
        "tccd", "lsd", "runningboardd", "softwareupdated", "textinputmenuagent", "talagent", "pboard",
        "fontd", "nsurlsessiond", "cloudd", "bird", "photoanalysisd", "watchdogd", "wallpaper",
        "windowmanager", "backgroundtaskmanagementagent", "accessibilityuiserver",
        // Windows
        "system", "registry", "smss", "csrss", "wininit", "winlogon", "services", "lsass", "svchost",
        "dwm", "explorer", "sihost", "fontdrvhost", "ctfmon", "runtimebroker", "searchhost",
        "startmenuexperiencehost", "shellexperiencehost", "textinputhost", "taskhostw", "dllhost",
        "conhost", "audiodg", "spoolsv", "msmpeng", "securityhealthservice", "memory compression",
        "vmmem", "vmmemwsl", "applicationframehost", "lockapp", "widgets", "secure system",
    ];
    if NAMES.contains(&n.as_str()) {
        return true;
    }
    let exe = p.exe.as_deref().unwrap_or("");
    exe.starts_with("/System/")
        || exe.starts_with("/usr/libexec/")
        || exe.starts_with("/usr/sbin/")
        || exe.starts_with("/sbin/")
        || exe.to_ascii_lowercase().starts_with("c:\\windows\\")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proc(name: &str, cmd: &[&str]) -> ProcInfo {
        ProcInfo {
            pid: 100,
            name: name.into(),
            cmd: cmd.iter().map(|s| s.to_string()).collect(),
            owned_by_user: true,
            ..Default::default()
        }
    }

    #[test]
    fn recognises_agent_sessions() {
        let c = classify_cli(&proc("claude", &["claude", "--resume"])).unwrap();
        assert_eq!(c, (Kind::AgentSession, "Claude Code".into()));
        let c = classify_cli(&proc("node", &["node", "/usr/local/lib/node_modules/@openai/codex/bin/codex.js"])).unwrap();
        assert_eq!(c.0, Kind::AgentSession);
        assert_eq!(c.1, "Codex");
        let c = classify_cli(&proc("node", &["node", "/opt/homebrew/bin/ggcoder"])).unwrap();
        assert_eq!(c.1, "GG Coder");
    }

    #[test]
    fn agent_serving_mcp_is_an_mcp_server() {
        let c = classify_cli(&proc("claude", &["claude", "mcp", "serve"])).unwrap();
        assert_eq!(c.0, Kind::McpServer);
    }

    #[test]
    fn recognises_mcp_servers_by_whole_word() {
        let c = classify_cli(&proc("node", &["node", "/x/node_modules/.bin/chrome-devtools-mcp"])).unwrap();
        assert_eq!(c, (Kind::McpServer, "chrome-devtools-mcp".into()));
        let c = classify_cli(&proc("npx", &["npx", "-y", "@playwright/mcp@latest"])).unwrap();
        assert_eq!(c, (Kind::McpServer, "playwright".into()));
        let c = classify_cli(&proc("node", &["node", "/x/@modelcontextprotocol/server-filesystem/dist/index.js"])).unwrap();
        assert_eq!(c, (Kind::McpServer, "filesystem".into()));
        assert!(classify_cli(&proc("node", &["node", "mcpherson.js"])).is_none());
    }

    #[test]
    fn recognises_dev_servers() {
        let c = classify_cli(&proc("node", &["node", "/p/node_modules/vite/bin/vite.js"])).unwrap();
        assert_eq!(c, (Kind::DevServer, "Vite".into()));
        let c = classify_cli(&proc("next-server (v15.1.0)", &["next-server (v15.1.0)"]));
        // Process titles with spaces still match on the leading word via ports or name.
        assert!(c.is_none() || c.unwrap().0 == Kind::DevServer);
        let c = classify_cli(&proc("python3.12", &["python3.12", "-m", "uvicorn", "app:app", "--reload"])).unwrap();
        assert_eq!(c.1, "Uvicorn");
    }

    #[test]
    fn unknown_runtime_with_open_port_is_a_dev_server() {
        let mut p = proc("node", &["node", "server.js"]);
        p.ports = vec![3000];
        assert_eq!(classify_cli(&p).unwrap(), (Kind::DevServer, "Node".into()));
        // Without a port it is just a process.
        p.ports.clear();
        assert!(classify_cli(&p).is_none());
    }

    #[test]
    fn watchers_need_watch_flag() {
        assert!(classify_cli(&proc("node", &["node", "/p/node_modules/typescript/bin/tsc"])).is_none());
        let c = classify_cli(&proc("node", &["node", "/p/node_modules/typescript/bin/tsc", "--watch"])).unwrap();
        assert_eq!(c.0, Kind::Watcher);
    }

    #[test]
    fn language_servers() {
        let c = classify_cli(&proc("rust-analyzer", &["/Users/me/.vscode/extensions/rust-analyzer"])).unwrap();
        assert_eq!(c.0, Kind::LanguageServer);
        let c = classify_cli(&proc("node", &["node", "/p/node_modules/typescript/lib/tsserver.js"])).unwrap();
        assert_eq!(c.0, Kind::LanguageServer);
    }

    #[test]
    fn wrappers_and_shells() {
        assert!(is_wrapper(&proc("npm", &["npm", "run", "dev"])));
        assert!(is_wrapper(&proc("npm run dev", &["npm run dev"])), "npm renames its process title");
        assert!(is_wrapper(&proc("node", &["npm run dev"])), "macOS reports npm as node");
        assert!(is_wrapper(&proc("node", &["node", "/opt/homebrew/lib/node_modules/npm/bin/npm-cli.js", "run", "dev"])));
        assert!(is_wrapper(&proc("sh", &["sh", "-c", "vite"])));
        assert!(!is_wrapper(&proc("zsh", &["-zsh"])));
        assert!(is_interactive_shell(&proc("-zsh", &["-zsh"])));
        assert!(!is_wrapper(&proc("node", &["node", "server.js"])));
    }

    #[test]
    fn app_roots() {
        assert_eq!(
            app_root("/Applications/Google Chrome.app/Contents/Frameworks/Google Chrome Framework.framework/Helpers/Google Chrome Helper (Renderer).app/Contents/MacOS/x").as_deref(),
            Some("/Applications/Google Chrome.app")
        );
        assert_eq!(app_root("/opt/homebrew/bin/node"), None);
        assert_eq!(
            app_root("C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe").as_deref(),
            Some("C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe")
        );
        assert_eq!(app_root("C:\\Program Files\\nodejs\\node.exe"), None);
        assert_eq!(app_root("C:\\Windows\\System32\\svchost.exe"), None);
        assert_eq!(app_name("/Applications/Google Chrome.app"), "Google Chrome");
        assert_eq!(app_name("C:\\x\\chrome.exe"), "Google Chrome");
        let sim = app_root("/Users/me/Library/Developer/CoreSimulator/Devices/X/data/Containers/Bundle/Application/Y/pew2.app/pew2").unwrap();
        assert_eq!(app_name(&sim), "iOS Simulator apps");
    }

    #[test]
    fn app_kinds() {
        assert_eq!(classify_app("Google Chrome"), Kind::Browser);
        assert_eq!(classify_app("Ghostty"), Kind::Terminal);
        assert_eq!(classify_app("Cursor"), Kind::Editor);
        assert_eq!(classify_app("Docker"), Kind::ContainerRuntime);
        assert_eq!(classify_app("Slack"), Kind::App);
    }

    #[test]
    fn os_critical() {
        assert!(is_os_critical(&proc("WindowServer", &[])));
        assert!(is_os_critical(&proc("explorer.exe", &[])));
        let mut p = proc("someagent", &[]);
        p.exe = Some("/System/Library/CoreServices/x".into());
        assert!(is_os_critical(&p));
        assert!(!is_os_critical(&proc("node", &[])));
    }

    // ---- Cases found with the detection lab (scripts/lab) ----

    fn with_exe(name: &str, exe: &str, cmd: &[&str]) -> ProcInfo {
        ProcInfo { exe: Some(exe.into()), ..proc(name, cmd) }
    }

    #[test]
    fn homebrew_python_is_command_line_work_not_an_app() {
        let exe = "/opt/homebrew/Cellar/python@3.14/3.14.6/Frameworks/Python.framework/Versions/3.14/Resources/Python.app/Contents/MacOS/Python";
        assert_eq!(app_root(exe), None, "Python.app must not swallow every python tool");
        // A real app is still an app.
        assert!(app_root("/Applications/Slack.app/Contents/MacOS/Slack").is_some());
        let mut uv = with_exe("Python", exe, &[exe, "-m", "uvicorn", "main:app", "--reload"]);
        uv.ports = vec![8000];
        assert_eq!(classify_cli(&uv).unwrap(), (Kind::DevServer, "Uvicorn".into()));
        let mut dj = with_exe("Python", exe, &[exe, "/x/proj/manage.py", "runserver"]);
        dj.ports = vec![8001];
        assert_eq!(classify_cli(&dj).unwrap(), (Kind::DevServer, "Django".into()));
        let jl = with_exe("Python", exe, &[exe, "-m", "jupyterlab"]);
        assert_eq!(classify_cli(&jl).unwrap().0, Kind::DevServer);
    }

    #[test]
    fn mobile_toolchains() {
        let fl = proc("dart", &["dart", "--packages=x", "/x/flutter/bin/cache/flutter_tools.snapshot", "run"]);
        assert_eq!(classify_cli(&fl).unwrap(), (Kind::DevServer, "Flutter".into()));
        let rn = proc("node", &["node", "/x/app/node_modules/react-native/cli.js", "start"]);
        assert_eq!(classify_cli(&rn).unwrap().0, Kind::DevServer);
        let gradle = proc("java", &["java", "-Xmx2g", "-cp", "/x/gradle-launcher-8.10.jar", "org.gradle.launcher.daemon.bootstrap.GradleDaemon", "8.10"]);
        assert_eq!(classify_cli(&gradle).unwrap(), (Kind::LanguageServer, "Gradle daemon".into()));
        let emu = with_exe("qemu-system-aarch64", "/Users/x/Library/Android/sdk/emulator/qemu/darwin-aarch64/qemu-system-aarch64", &["qemu-system-aarch64", "-avd", "Pixel_8"]);
        assert_eq!(classify_cli(&emu).unwrap(), (Kind::LocalModel, "Android emulator".into()));
        // Some other qemu (a homebrew VM) is not an Android emulator.
        let vm = with_exe("qemu-system-aarch64", "/opt/homebrew/bin/qemu-system-aarch64", &["qemu-system-aarch64"]);
        assert!(classify_cli(&vm).is_none_or(|c| c.1 != "Android emulator"));
    }

    #[test]
    fn programs_you_built_listening_on_a_port() {
        let mut go = with_exe("api", "/Users/x/Library/Caches/go-build/b001/exe/api", &["api"]);
        go.ports = vec![8080];
        assert_eq!(classify_cli(&go).unwrap(), (Kind::DevServer, "api".into()));
        let mut rs = with_exe("server", "/Users/x/proj/target/debug/server", &["server"]);
        rs.ports = vec![3000];
        assert_eq!(classify_cli(&rs).unwrap().0, Kind::DevServer);
        // Installed apps and system services on a port are not "dev builds".
        let mut pg = with_exe("postgres", "/opt/homebrew/opt/postgresql@17/bin/postgres", &["postgres"]);
        pg.ports = vec![5432];
        assert!(classify_cli(&pg).is_none());
        let mut ctl = with_exe("ControlCenter", "/System/Library/CoreServices/ControlCenter.app/Contents/MacOS/ControlCenter", &["ControlCenter"]);
        ctl.ports = vec![5000];
        assert!(classify_cli(&ctl).is_none());
        // Without a port, a built binary is just a program.
        let quiet = with_exe("tool", "/Users/x/proj/target/release/tool", &["tool"]);
        assert!(classify_cli(&quiet).is_none());
    }

    #[test]
    fn gg_coder_by_package_name() {
        let gg = proc("node", &["/opt/homebrew/bin/node", "/x/node_modules/@kenkaiiii/ggcoder/dist/cli.js"]);
        assert_eq!(classify_cli(&gg).unwrap(), (Kind::AgentSession, "GG Coder".into()));
    }

    #[test]
    fn mcp_servers_are_named_by_their_package() {
        let via_npx = proc("npx", &["npx", "-y", "@playwright/mcp@latest"]);
        assert_eq!(classify_cli(&via_npx).unwrap(), (Kind::McpServer, "playwright".into()));
        let child = proc("node", &["/opt/homebrew/bin/node", "/x/node_modules/@playwright/mcp/cli.js"]);
        assert_eq!(classify_cli(&child).unwrap(), (Kind::McpServer, "playwright".into()));
        let npm_exec = proc("npm", &["npm exec @playwright/mcp", "-e", "x"]);
        assert_eq!(classify_cli(&npm_exec).unwrap(), (Kind::McpServer, "playwright".into()));
        assert_eq!(classify_cli(&child).unwrap(), (Kind::McpServer, "playwright".into()));
        let ctx7 = proc("node", &["node", "/x/node_modules/@upstash/context7-mcp/dist/index.js"]);
        assert_eq!(classify_cli(&ctx7).unwrap(), (Kind::McpServer, "context7-mcp".into()));
    }

    #[test]
    fn unknown_servers_are_named_by_runtime_not_a_random_arg() {
        let mut deno = proc("deno", &["deno", "run", "-A", "--watch", "main.ts"]);
        deno.ports = vec![8000];
        assert_eq!(classify_cli(&deno).unwrap(), (Kind::DevServer, "Deno".into()));
        let mut php = proc("php", &["php", "-S", "127.0.0.1:8006"]);
        php.ports = vec![8006];
        assert_eq!(classify_cli(&php).unwrap(), (Kind::DevServer, "PHP".into()));
        let mut node = proc("node", &["node", "server.js"]);
        node.ports = vec![3000];
        assert_eq!(classify_cli(&node).unwrap(), (Kind::DevServer, "Node".into()));
        let rr = proc("node", &["node", "/x/node_modules/@react-router/dev/bin.js", "dev"]);
        assert_eq!(classify_cli(&rr).unwrap(), (Kind::DevServer, "React Router".into()));
    }
}
