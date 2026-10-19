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
    "uv", "uvx", "npx", "bunx", "pnpx",
];

pub fn is_runtime(name: &str) -> bool {
    let n = base(name);
    RUNTIMES.contains(&n.as_str()) || (n.starts_with("python3.") && n.len() <= 10)
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
    ("react-router", "React Router"),
    ("react-scripts", "Create React App"),
    ("webpack-dev-server", "webpack"),
    ("webpack", "webpack"),
    ("parcel", "Parcel"),
    ("gatsby", "Gatsby"),
    ("storybook", "Storybook"),
    ("expo", "Expo"),
    ("metro", "Metro"),
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
    ("jupyter-notebook", "Jupyter"),
    ("jupyter", "Jupyter"),
    ("manage.py", "Django"),
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
    let program = p.cmd.first().map(|a| stem(a)).unwrap_or_else(|| stem(&p.name));
    out.push(program.clone());
    // Only a runtime's *script* (or `python -m module`) names the real tool.
    // Later args are data (`--model gemini`) and must not change the verdict.
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
            if clean == "mcp" && i > 0 && segs[i - 1].starts_with('@') {
                return Some(segs[i - 1].trim_start_matches('@').to_string());
            }
            return Some(strip_ext(clean));
        }
    }
    None
}

fn is_mcp(p: &ProcInfo) -> Option<String> {
    for a in p.cmd.iter().take(4) {
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
    if !p.ports.is_empty() && is_runtime(&p.name) {
        let script = p
            .cmd
            .iter()
            .skip(1)
            .find(|a| !a.starts_with('-'))
            .map(|a| base(a))
            .unwrap_or_else(|| base(&p.name));
        return Some((Kind::DevServer, script));
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
        assert_eq!(classify_cli(&p).unwrap(), (Kind::DevServer, "server.js".into()));
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
}
