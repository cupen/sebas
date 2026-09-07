"""Invoke tasks for sebas.

Usage:
    invoke build-image              # build image
    invoke build-image --push       # build + push to ghcr.io
    invoke testsuite-webui-sandbox         # throwaway webui backend for GUI testing
    invoke testsuite-webui-sandbox --auth  # same, with admin/admin login
    invoke testsuite-webui                 # Playwright browser suite (sandboxed)
    invoke --help                   # list all tasks
"""

import os
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
import urllib.request
from invoke import task

PROJECT = "sebas"
IMAGE = f"ghcr.io/cupen/{PROJECT}"


@task(
    help={
        "tag": "Image tag (default: latest)",
        "push": "Push to registry after building",
        "no-cache": "Disable layer cache",
    }
)
def build_image(c, tag="latest", push=False, no_cache=False):
    """Build the Docker image."""
    image_tag = f"{IMAGE}:{tag}"
    cmd = ["docker", "build"]
    if no_cache:
        cmd.append("--no-cache")
    cmd.extend(["-t", image_tag, "-f", "Dockerfile", "."])

    print(f"🐳 Building {image_tag} ...")
    result = c.run(" ".join(cmd), pty=True, echo=True)
    if result.failed:
        print(f"❌ Build failed")
        raise SystemExit(1)

    print(f"✅ Built: {image_tag}")

    if push:
        push_image(c, tag)


@task(help={"tag": "Image tag to push (default: latest)"})
def push_image(c, tag="latest"):
    """Push the image to ghcr.io."""
    image_tag = f"{IMAGE}:{tag}"
    print(f"📤 Pushing {image_tag} ...")
    result = c.run(f"docker push {image_tag}", pty=True, echo=True)
    if result.failed:
        print(f"❌ Push failed")
        raise SystemExit(1)
    print(f"✅ Pushed: {image_tag}")


@task
def clean(c):
    """Remove built Docker images."""
    image_tag = f"{IMAGE}:latest"
    c.run(f"docker rmi {image_tag} 2>/dev/null || true", echo=True)
    print(f"🧹 Cleaned: {image_tag}")

@task(help={"case": "Run a single testsuite-e2e case by name (cargo test filter)"})
def testsuite_e2e(c, case=None):
    """Build the workspace and run the process-level core-flow suite (testsuite-e2e)."""
    print("Building workspace (sebas + fake-claude) ...")
    result = c.run("cargo build", echo=True)
    if result.failed:
        raise SystemExit(1)
    test_filter = f"{case} " if case else ""
    cmd = f"cargo test --test testsuite_e2e_test {test_filter}-- --ignored".replace("  ", " ")
    result = c.run(cmd, echo=True)
    if result.failed:
        print("e2e suite FAILED; kept sandbox dirs are printed above (or under target/tests/)")
        raise SystemExit(1)


@task(help={"case": "Run a single acceptance journey by name (cargo test filter)"})
def testsuite_acceptance(c, case=None):
    """Build the workspace and run the acceptance suite (journey-level)."""
    print("Building workspace (sebas + fake-claude) ...")
    result = c.run("cargo build", echo=True)
    if result.failed:
        raise SystemExit(1)
    test_filter = f"{case} " if case else ""
    cmd = f"cargo test --test testsuite_acceptance_test {test_filter}-- --ignored".replace("  ", " ")
    result = c.run(cmd, echo=True)
    if result.failed:
        print("acceptance suite FAILED; kept sandbox dirs are printed above (or under target/tests/)")
        raise SystemExit(1)


def _testsuite_webui_spec_structure(c):
    """Spec-structure gate (converge-webui-e2e-tree): every test() must live
    inside a test.describe(). A top-level bare `test(` (column 0) means a new
    case bypassed the functional tree — refuse to run and point at the file."""
    import glob as _glob

    offenders = []
    for path in sorted(_glob.glob("tests/testsuite-webui/tests/*.spec.ts")):
        with open(path, encoding="utf-8") as f:
            for lineno, line in enumerate(f, 1):
                if line.startswith("test("):
                    offenders.append(f"{path}:{lineno}")
    if offenders:
        print("❌ spec structure violation — top-level bare test() outside test.describe():")
        for o in offenders:
            print(f"   {o}")
        print("   Wrap the case in the functional tree: describe('<大功能>', () => describe('<子功能>', ...))")
        raise SystemExit(1)


def _testsuite_webui_preflight(c):
    """pnpm + Playwright chromium preflight; installs what is missing."""
    result = c.run("pnpm --version", hide=True, warn=True)
    if result.failed:
        print("❌ pnpm is required (corepack enable or install pnpm)")
        raise SystemExit(1)
    suite_dir = "tests/testsuite-webui"
    if not os.path.isdir(os.path.join(suite_dir, "node_modules")):
        print("Installing tests/testsuite-webui dependencies ...")
        if c.run(f"pnpm install --dir {suite_dir}", echo=True).failed:
            raise SystemExit(1)
    result = c.run(
        f"pnpm --dir {suite_dir} exec playwright install chromium", echo=True, warn=True
    )
    # Already-installed browsers exit 0; a failed download blocks the run.
    if result.failed and "already installed" not in (result.stdout or ""):
        raise SystemExit(1)


@task(help={"case": "Run a single webui journey (spec file stem: first-paint, auth, ...)"})
def testsuite_webui(c, case=None):
    """Build + run the browser-level webui suite (testsuite-webui, Playwright, sandboxed)."""
    # Spec-structure gate first: refuse tree-bypassing cases before any build.
    _testsuite_webui_spec_structure(c)

    # Frontend dist is baked into the binary at build time — rebuild it when
    # the frontend sources are newer than the last dist build.
    dist_index = "sebas-webui/frontend/dist/index.html"
    need_dist = not os.path.exists(dist_index)
    if not need_dist:
        dist_mtime = os.path.getmtime(dist_index)
        src_root = "sebas-webui/frontend/src"
        for root, _dirs, files in os.walk(src_root):
            for f in files:
                if f.endswith((".ts", ".css", ".html")):
                    if os.path.getmtime(os.path.join(root, f)) > dist_mtime:
                        need_dist = True
                        break
            if need_dist:
                break
    if need_dist:
        print("Building frontend dist (sources newer than dist) ...")
        if c.run("pnpm run --dir sebas-webui/frontend build", echo=True).failed:
            raise SystemExit(1)
    else:
        print("frontend dist up to date")

    print("Building workspace (sebas + fake-claude) ...")
    if c.run("cargo build --bin sebas --bin fake-claude", echo=True).failed:
        raise SystemExit(1)

    _testsuite_webui_preflight(c)

    suite_dir = "tests/testsuite-webui"
    if case:
        # --case auth runs the auth-on form; anything else filters the main suite.
        if case == "auth":
            cmd = f"pnpm --dir {suite_dir} exec playwright test --config playwright.auth.config.ts"
        else:
            cmd = f"pnpm --dir {suite_dir} exec playwright test {case}"
    else:
        cmd = f"pnpm --dir {suite_dir} exec playwright test && pnpm --dir {suite_dir} exec playwright test --config playwright.auth.config.ts"
    result = c.run(cmd, echo=True)
    if result.failed:
        print(
            "testsuite-webui FAILED — sandbox scene kept for debugging (path printed above)."
        )
        print("Reuse it interactively: TESTSUITE_REUSE=1 TESTSUITE_KEEP=1 invoke testsuite-webui --case <name>")
        raise SystemExit(1)


# ---------------------------------------------------------------------------
# webui sandbox harness: one implementation serves humans and Playwright.
#
# Replaces scripts/webui_e2e_server.sh + scripts/test_webui_sandbox.sh (both
# removed): the same throwaway-dir + sandboxed-env + core --router --debug
# --webui assembly backs `invoke testsuite-webui-sandbox` (human, foreground,
# friendly output) and `invoke testsuite-webui-server` (blocking server process for
# Playwright's webServer.command). Contract with
# tests/testsuite-webui/tests/reporters/keep-on-fail.ts (authoritative keep/clean
# owner): the TESTSUITE_SCENE_FILE pointer + the `.tests-failed` marker.
# ---------------------------------------------------------------------------

_TESTSUITE_PORT = 9899
_TESTSUITE_AUTH_PORT = 9898
_TESTSUITE_HUMAN_PORT = 9879


def _sandbox_bin(name):
    """Repo-built binary; native Windows processes need the .exe suffix."""
    suffix = ".exe" if os.name == "nt" or sys.platform.startswith(("msys", "cygwin")) else ""
    return os.path.join("target", "debug", name + suffix)


def _cfg_path(path):
    """Path as written into config.toml. TOML basic strings treat `\` as an
    escape, so separators must come out as forward slashes; msys/cygwin Python
    additionally produces POSIX paths a native .exe cannot parse, so convert
    via cygpath when present."""
    if sys.platform.startswith(("msys", "cygwin")) and shutil.which("cygpath"):
        out = subprocess.run(["cygpath", "-m", path], capture_output=True, text=True)
        if out.returncode == 0 and out.stdout.strip():
            return out.stdout.strip()
    return path.replace("\\", "/")


def _sandbox_env(work):
    """Full env isolation: every default that would fall back to the real
    ~/.sebas is redirected into the throwaway dir."""
    env = dict(os.environ)
    env.update(
        {
            "SEBAS_CORE_SECRET": "fake",
            "SEBAS_STATE_DB": os.path.join(work, "sebas.db"),
            "SEBAS_STATE_FILE": os.path.join(work, "state.json"),
            "SEBAS_ROUTER_PROVIDER_OVERLAY": os.path.join(work, "providers.json"),
            "SEBAS_WEBUI_AUTH_FILE": os.path.join(work, "webui-auth.json"),
            "SEBAS_PROJECTS_PATH": os.path.join(work, "projects.json"),
        }
    )
    return env


def _write_sandbox_config(work, fake_bin, auth_on):
    """config.toml following the AGENTS.md debug recipe."""
    cfg = _cfg_path(work)
    fake = _cfg_path(os.path.abspath(fake_bin))
    auth_toml = "true" if auth_on else "false"
    text = f"""[feishu]
enabled = false

[acp.agents.claude]
driver = "claude"
path = "{fake}"
sessions_dir = "{cfg}/claude-sessions"
work_dir = "{cfg}/work"

[dispatch]
state_file = "{cfg}/sessions.json"

[media]
download_dir = "{cfg}/downloads"

[watchdog.core]
channel_path = "{cfg}/core-channel.sock"

[watchdog.webui]
enabled = false
auth = {auth_toml}

# router validate requires >=1 provider with a base_url; the debug `test`
# provider is injected by --debug, this dummy never dials anything.
[provider.anthropic]
api_key = "sk-sandbox-dummy"

[router]
provider_overlay = "{cfg}/providers.json"
usage_file = "{cfg}/router-usage.jsonl"
"""
    with open(os.path.join(work, "config.toml"), "w") as f:
        f.write(text)


def _health_ok(url):
    try:
        with urllib.request.urlopen(url, timeout=3) as r:
            return r.status == 200
    except Exception:
        return False


def _run_webui_sandbox(port, auth_on, keep, reuse, human):
    """Assemble a throwaway backend and block until signalled; then clean up.
    Mirrors the retired bash harness semantics (ports, env names, pointer and
    marker contracts) so Playwright configs and the reporter keep working."""
    sebas_bin = _sandbox_bin("sebas")
    fake_bin = _sandbox_bin("fake-claude")
    for path in (sebas_bin, fake_bin):
        if not (os.path.isfile(path) and os.access(path, os.X_OK)):
            print(f"error: {path} missing, run: cargo build --bin sebas --bin fake-claude")
            raise SystemExit(1)

    scene_file = os.environ.get(
        "TESTSUITE_SCENE_FILE", os.path.join(tempfile.gettempdir(), f"sebas-testsuite-webui-scene-{port}")
    )
    health_url = f"http://127.0.0.1:{port}/health"
    stop = threading.Event()

    def _on_signal(signum, frame):
        stop.set()

    signal.signal(signal.SIGTERM, _on_signal)
    signal.signal(signal.SIGINT, _on_signal)

    # Reuse mode: a kept scene still serving on this port is handed over.
    # NOTE: flush=True throughout — stdout is a pipe/file under Playwright's
    # webServer (block-buffered by default); readiness lines must land now.
    if reuse:
        try:
            with open(scene_file) as f:
                kept = f.read().strip()
        except OSError:
            kept = ""
        if kept and _health_ok(health_url):
            print(f"[testsuite] reusing existing sandbox on port {port} (dir: {kept})", flush=True)
            while not stop.is_set():
                stop.wait(1.0)
            return

    work = tempfile.mkdtemp(prefix="sbtestsuite.")
    for sub in ("media", "acp", "work"):
        os.makedirs(os.path.join(work, sub), exist_ok=True)
    with open(scene_file, "w") as f:
        f.write(work)
    _write_sandbox_config(work, fake_bin, auth_on)

    if auth_on:
        result = subprocess.run(
            [sebas_bin, "webui-passwd", "--user", "admin", "--password-stdin"],
            input=b"admin",
            env={**os.environ, "SEBAS_WEBUI_AUTH_FILE": os.path.join(work, "webui-auth.json")},
        )
        if result.returncode != 0:
            print("error: provisioning admin/admin failed", flush=True)
            raise SystemExit(1)

    log_path = os.path.join(work, "core.log")
    log = open(log_path, "w")
    proc = subprocess.Popen(
        [
            sebas_bin, "core", "-c", os.path.join(work, "config.toml"),
            "--router", "--debug", "--webui", "--webui-port", str(port),
        ],
        env=_sandbox_env(work),
        stdout=log,
        stderr=subprocess.STDOUT,
    )

    def _log_tail(n=30):
        try:
            with open(log_path) as f:
                return "".join(f.readlines()[-n:])
        except OSError:
            return ""

    # Readiness poll; a dead child during startup is an immediate error.
    for _ in range(120):
        if _health_ok(health_url):
            break
        if proc.poll() is not None:
            print("error: sebas core exited during startup; log:", flush=True)
            print(_log_tail(), flush=True)
            raise SystemExit(1)
        time.sleep(0.5)
    else:
        print("error: sandbox not healthy after 60s; log:", flush=True)
        print(_log_tail(), flush=True)
        proc.terminate()
        raise SystemExit(1)
    print(f"[testsuite] sandbox ready on port {port} (dir: {work})", flush=True)
    if human:
        if auth_on:
            print(f"### webui 沙箱就绪：http://127.0.0.1:{port}/  （鉴权开启，admin / admin）", flush=True)
        else:
            print(f"### webui 沙箱就绪：http://127.0.0.1:{port}/  （鉴权关闭，免登录）", flush=True)
        print(f"### 日志与状态均在 {work}（core.log）；Ctrl-C 退出并清理", flush=True)

    while proc.poll() is None and not stop.is_set():
        stop.wait(0.5)

    # Teardown: SIGTERM the backend, then keep-or-clean (the reporter owns the
    # authoritative decision via `.tests-failed`; this is the fallback path).
    if proc.poll() is None:
        proc.terminate()
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            proc.kill()
    log.close()
    failed_marker = os.path.exists(os.path.join(work, ".tests-failed"))
    if failed_marker or keep:
        print(f"[testsuite] sandbox scene kept at: {work} (core.log inside)", flush=True)
    else:
        shutil.rmtree(work, ignore_errors=True)
        try:
            os.remove(scene_file)
        except OSError:
            pass
        print(f"[testsuite] sandbox cleaned: {work}", flush=True)


@task(
    help={
        "port": "webui port (default: 9879)",
        "auth": "enable auth with test account admin/admin",
        "keep": "keep the sandbox dir on exit",
    }
)
def testsuite_webui_sandbox(c, port=None, auth=False, keep=False):
    """Run a throwaway webui backend for GUI testing (foreground, Ctrl-C to stop)."""
    _run_webui_sandbox(
        port=int(port) if port else _TESTSUITE_HUMAN_PORT,
        auth_on=bool(auth),
        keep=bool(keep),
        reuse=bool(os.environ.get("TESTSUITE_REUSE")),
        human=True,
    )


@task
def testsuite_webui_server(c):
    """Blocking sandbox server for Playwright's webServer.command (ports 9899/9898)."""
    auth_on = os.environ.get("TESTSUITE_AUTH", "0") == "1"
    default_port = _TESTSUITE_AUTH_PORT if auth_on else _TESTSUITE_PORT
    _run_webui_sandbox(
        port=int(os.environ.get("TESTSUITE_PORT", default_port)),
        auth_on=auth_on,
        keep=os.environ.get("TESTSUITE_KEEP", "0") == "1",
        reuse=os.environ.get("TESTSUITE_REUSE", "0") == "1",
        human=os.environ.get("TESTSUITE_HUMAN", "0") == "1",
    )


@task(
    help={
        "source": "sebas_artifact_source: release (default) | file | preinstalled",
        "port": "sebas_webui_port override",
        "extra": "Extra args passed through to ansible-playbook",
    }
)
def deploy(c, source=None, port=None, extra=""):
    """Deploy via ansible (default local inventory; needs a Linux/WSL control machine)."""
    overrides = ""
    if source:
        overrides += f" -e sebas_artifact_source={source}"
    if port:
        overrides += f" -e sebas_webui_port={port}"
    passthrough = f" {extra}" if extra else ""
    cmd = f"ansible-playbook site.yml{overrides}{passthrough}"
    result = c.run(cmd, cd="ansible", echo=True)
    if result.failed:
        print("❌ deploy failed")
        raise SystemExit(1)
    print("✅ deployed")
