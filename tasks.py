"""Invoke tasks for sebas.

Usage:
    invoke build-image              # build image
    invoke build-image --push       # build + push to ghcr.io
    invoke testsuite-webui-sandbox         # throwaway webui backend for GUI testing
    invoke testsuite-webui-sandbox --auth  # same, with admin/admin login
    invoke testsuite-webui                 # Playwright browser suite (sandboxed)
    invoke --help                   # list all tasks
"""

import json
import os
import re
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


def _cleanup_stale_sandboxes():
    """Remove leftover sbtestsuite.* dirs from crashed/aborted runs.

    Scans tempfile.gettempdir() for dirs named sbtestsuite.*, checks if the
    recorded PIDs (pids.json) are still alive, and removes the dir if not.
    This is the safety net for SIGKILL / hard-crashed test processes that
    never reached their teardown code. Safe to call before/after any suite.
    """
    tmp = tempfile.gettempdir()
    removed = []
    for name in os.listdir(tmp):
        if not name.startswith("sbtestsuite."):
            continue
        d = os.path.join(tmp, name)
        if not os.path.isdir(d):
            continue
        pids_file = os.path.join(d, "pids.json")
        # A failed suite keeps this dir for postmortem — its recorded pids are
        # dead precisely BECAUSE teardown killed them, so the liveness check
        # below would sweep it away within the same invocation. Exempt it.
        if os.path.exists(os.path.join(d, ".tests-failed")):
            continue
        alive = False
        try:
            with open(pids_file) as f:
                for pid in json.load(f).values():
                    if isinstance(pid, int):
                        try:
                            os.kill(pid, 0)
                            alive = True
                            break
                        except ProcessLookupError:
                            pass
        except (OSError, ValueError, PermissionError):
            pass
        if not alive:
            shutil.rmtree(d, ignore_errors=True)
            removed.append(d)
    if removed:
        print(f"[cleanup] removed {len(removed)} stale sandbox dirs", flush=True)
    _sweep_orphan_test_processes()


def _sweep_orphan_test_processes():
    """Kill orphaned sebas processes leaked by crashed suite runs (sebas-gc7).

    Two leak classes, two rules:

    - Rust suites (`target/tests/sebas/testsuite_*` in cmdline): the dir only
      exists while a suite runs and cargo's target lock serializes suite
      runs, so any live process carrying one is leaked — kill.
    - webui sandbox (`sbtestsuite.` in cmdline): the dir survives for active
      runs AND intentionally-kept failure scenes, so only kill when the
      referenced dir is already gone — the surest sign the harness died and
      teardown never ran (an active run's dir always exists).

    SIGTERM first so cores exit gracefully, SIGKILL stragglers. The
    operator's real instance (AppImage / cargo bin, ~/.sebas config) never
    references either test path.
    """
    if not os.path.isdir("/proc"):
        return
    rust_marker = "target/tests/sebas/testsuite_"
    targets = []
    for entry in os.listdir("/proc"):
        if not entry.isdigit():
            continue
        try:
            with open(f"/proc/{entry}/cmdline", "rb") as f:
                cmd = f.read().replace(b"\0", b" ").decode(errors="replace")
        except OSError:
            continue
        if "sebas" not in cmd:
            continue
        if rust_marker in cmd:
            targets.append(int(entry))
            continue
        m = re.search(r"(\S*sbtestsuite\.\S+)/", cmd)
        if m and not os.path.isdir(m.group(1)):
            targets.append(int(entry))
    if not targets:
        return
    for pid in targets:
        try:
            os.kill(pid, signal.SIGTERM)
        except OSError:
            pass
    deadline = time.time() + 3
    alive = targets
    while time.time() < deadline:
        alive = [p for p in targets if os.path.exists(f"/proc/{p}")]
        if not alive:
            return
        time.sleep(0.2)
    for pid in alive:
        try:
            os.kill(pid, signal.SIGKILL)
        except OSError:
            pass
    print(f"[cleanup] killed orphaned test processes: {targets}", flush=True)



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
    _cleanup_stale_sandboxes()
    try:
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
    finally:
        _cleanup_stale_sandboxes()


@task(help={"case": "Run a single real-agent journey by name (cargo test filter: real_opencode / real_claude)"})
def testsuite_real_agents(c, case=None):
    """Build the workspace and run the real-backend agent journeys (real_agent_e2e).

    Unlike testsuite-e2e these drive REAL agent CLIs (opencode acp, claude) and
    cost real LLM tokens (10-120s per turn); backends that are missing or not
    authenticated self-skip with a printed reason. Serialized
    (--test-threads=1) to keep the one-scene-per-journey sandbox predictable.
    """
    print("Building workspace (sebas + fake-claude) ...")
    result = c.run("cargo build", echo=True)
    if result.failed:
        raise SystemExit(1)
    test_filter = f"{case} " if case else ""
    cmd = f"cargo test --test real_agent_e2e_test {test_filter}-- --ignored --test-threads=1".replace("  ", " ")
    result = c.run(cmd, echo=True)
    if result.failed:
        print("real-agent suite FAILED; kept scene dirs are printed above (or under target/tests/)")
        raise SystemExit(1)


@task(help={"case": "Run a single acceptance journey by name (cargo test filter)"})
def testsuite_acceptance(c, case=None):
    """Build the workspace and run the acceptance suite (journey-level)."""
    _cleanup_stale_sandboxes()
    try:
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
    finally:
        _cleanup_stale_sandboxes()


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
    _cleanup_stale_sandboxes()
    try:
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

        print("Building workspace (sebas + fakes) ...")
        if (
            c.run(
                "cargo build -p sebas -p sebas-acp --bin sebas --bin fake-claude --bin fake-acp-agent",
                echo=True,
            ).failed
        ):
            raise SystemExit(1)

        _testsuite_webui_preflight(c)

        suite_dir = "tests/testsuite-webui"
        if case:
            # --case auth runs the auth-on form; --case deployment or
            # --case approval-detached the detached dual-process topology
            # (core + standalone webui, no shared-secret env); anything else
            # filters the main suite.
            if case == "auth":
                cmd = f"pnpm --dir {suite_dir} exec playwright test --config playwright.auth.config.ts"
            elif case in ("deployment", "approval-detached"):
                cmd = (
                    f"pnpm --dir {suite_dir} exec playwright test --config playwright.detached.config.ts"
                    f" {case}"
                )
            else:
                cmd = f"pnpm --dir {suite_dir} exec playwright test {case}"
        else:
            cmd = (
                f"pnpm --dir {suite_dir} exec playwright test"
                f" && pnpm --dir {suite_dir} exec playwright test --config playwright.auth.config.ts"
                f" && pnpm --dir {suite_dir} exec playwright test --config playwright.detached.config.ts"
            )
        result = c.run(cmd, echo=True)
        if result.failed:
            print(
                "testsuite-webui FAILED — sandbox scene kept for debugging (path printed above)."
            )
            print("Reuse it interactively: TESTSUITE_REUSE=1 TESTSUITE_KEEP=1 invoke testsuite-webui --case <name>")
            raise SystemExit(1)
    finally:
        _cleanup_stale_sandboxes()



# ---------------------------------------------------------------------------
# webui sandbox harness: one implementation serves humans and Playwright.
#
# Replaces scripts/webui_e2e_server.sh + scripts/test_webui_sandbox.sh (both
# removed): the same throwaway-dir + sandboxed-env + core --webui + 独立
# `sebas router --config … --debug` 两进程装配 backs `invoke
# testsuite-webui-sandbox` (human, foreground, friendly output) and `invoke
# testsuite-webui-server` (blocking server process for
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
    r"""Path as written into config.toml. TOML basic strings treat `\` as an
    escape, so separators must come out as forward slashes; msys/cygwin Python
    additionally produces POSIX paths a native .exe cannot parse, so convert
    via cygpath when present."""
    if sys.platform.startswith(("msys", "cygwin")) and shutil.which("cygpath"):
        out = subprocess.run(["cygpath", "-m", path], capture_output=True, text=True)
        if out.returncode == 0 and out.stdout.strip():
            return out.stdout.strip()
    return path.replace("\\", "/")


def _sandbox_env(work, secret=True):
    """Full env isolation: every default that would fall back to the real
    ~/.sebas is redirected into the throwaway dir. `secret=False` (detached
    dual-process mode) omits SEBAS_CORE_SECRET entirely — the core then
    auto-arms from a generated key file and clients discover it (D2/D3,
    harden-core-channel-deployment 5.4)."""
    env = dict(os.environ)
    env.update(
        {
            "SEBAS_STATE_DB": os.path.join(work, "sebas.db"),
            "SEBAS_STATE_FILE": os.path.join(work, "state.json"),
            "SEBAS_ROUTER_PROVIDER_OVERLAY": os.path.join(work, "providers.json"),
            # WebUI 用户库（SQLite；add-webui-multiuser-rbac）：bootstrap_auth
            # 与 webui-passwd 都从这里取路径。auth 关闭时无人打开它，设着只是
            # 让任何 stray 的建户调用也落不进真实 ~/.sebas。
            "SEBAS_WEBUI_AUTH_DB": os.path.join(work, "auth.db"),
            "SEBAS_PROJECTS_PATH": os.path.join(work, "projects.json"),
            # archive.json derives from SEBAS_HOME ($HOME/.sebas) — without
            # this the sandbox READ AND REWROTE the operator's real archive
            # (test sessions landed in it, 93-entry pollution, 2026-09-12).
            "SEBAS_HOME": work,
            "SEBAS_ARCHIVE_PATH": os.path.join(work, "archive.json"),
        }
    )
    if secret:
        env["SEBAS_CORE_SECRET"] = "fake"
    else:
        env.pop("SEBAS_CORE_SECRET", None)
    return env


def _write_sandbox_config(work, fake_bin, auth_on, webui_enabled=False, port=None, fake_acp_bin=None):
    """config.toml following the AGENTS.md debug recipe.

    `[watchdog.webui] enabled` decides the topology: false (default) = the
    bare core owns the webui via `--webui-port`; true = the standalone
    `sebas webui` serves it from host/port in this section (the detached
    dual-process form, harden-core-channel-deployment 5.4).

    `[acp.agents.fakeacp]` wires the generic-ACP fake (sebas-acp's
    fake-acp-agent) as an extra agent kind so model-selection journeys
    (cover-core-channel-test-gaps B2.2) can drive configOptions reflection:
    it advertises `bad-model`/`ok-model` (initial = first) and rejects `set_config_option` for
    `bad-model` (typed rejection; the claude driver/fake remain model-less —
    that terminal-teardown contract is pinned by the existing models.spec 3.2)."""
    cfg = _cfg_path(work)
    fake = _cfg_path(os.path.abspath(fake_bin))
    fake_acp = _cfg_path(os.path.abspath(fake_acp_bin)) if fake_acp_bin else None
    auth_toml = "true" if auth_on else "false"
    if webui_enabled:
        webui_toml = f'enabled = true\nhost = "127.0.0.1"\nport = {port}'
    else:
        webui_toml = "enabled = false"
    fakeacp_toml = ""
    if fake_acp:
        fakeacp_toml = f"""
[acp.agents.fakeacp]
driver = "acp"
command = ["{fake_acp}", "--journal", "{cfg}/fakeacp-journal.jsonl", "--model-options", "bad-model,ok-model", "--reject-model", "bad-model"]
"""
    text = f"""[feishu]
enabled = false

[acp.agents.claude]
driver = "claude"
path = "{fake}"
# workbench-turn-queue：慢档让 turn 在内容帧之后停留 800ms（driver 的
# watchdog 探测超时 1.5s，800ms 同步 sleep 仍可应答）——浏览器旅程因此有
# 确定性的 WORKING 窗口可提交忙中消息；其余旅程的轮询超时远大于此，无感。
args = ["--slow-ms", "800"]
sessions_dir = "{cfg}/claude-sessions"
work_dir = "{cfg}/work"
{fakeacp_toml}
[dispatch]
state_file = "{cfg}/sessions.json"

[media]
download_dir = "{cfg}/downloads"

[watchdog.core]
channel_path = "{cfg}/core-channel.sock"

[watchdog.webui]
{webui_toml}
auth = {auth_toml}

# router validate requires >=1 provider with a base_url; the debug `test`
# provider is injected by --debug, this dummy never dials anything.
[provider.anthropic]
api_key = "sk-sandbox-dummy"

# router 只以独立进程运行（unify-router-process-shape）：listen 默认
# 8787 是固定值，会撞操作员实例的托管 router——沙箱钉一个专用端口。
[router]
listen = "127.0.0.1:8791"
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


def _run_webui_sandbox(port, auth_on, keep, reuse, human, detached=False):
    """Assemble a throwaway backend and block until signalled; then clean up.
    Mirrors the retired bash harness semantics (ports, env names, pointer and
    marker contracts) so Playwright configs and the reporter keep working.

    `detached=True` is the dual-process topology (harden-core-channel-deployment
    5.4): the core runs WITHOUT `--webui` and a standalone `sebas webui` serves
    the dashboard — no SEBAS_CORE_SECRET anywhere (auto-arm + secret-file
    discovery). The two pids are published at `<scene>/pids.json` so the
    Playwright fixture (tests/helpers/detached.ts) can stop/start the core
    mid-journey; teardown kills whatever pids the file lists last."""
    sebas_bin = os.path.abspath(_sandbox_bin("sebas"))
    fake_bin = _sandbox_bin("fake-claude")
    fake_acp_bin = _sandbox_bin("fake-acp-agent")
    _cleanup_stale_sandboxes()
    for path in (sebas_bin, fake_bin, fake_acp_bin):
        if not (os.path.isfile(path) and os.access(path, os.X_OK)):
            print(
                f"error: {path} missing, run: cargo build -p sebas -p sebas-acp"
                " --bin sebas --bin fake-claude --bin fake-acp-agent"
            )
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
    _write_sandbox_config(work, fake_bin, auth_on, webui_enabled=detached, port=port, fake_acp_bin=fake_acp_bin)

    if auth_on:
        # 统一测试账号 admin/admin：webui-passwd 写沙箱内 auth.db（首个用户
        # 默认 root），建户先于进程拉起、失败即退（fail-fast，不等 health）。
        result = subprocess.run(
            [sebas_bin, "webui-passwd", "--user", "admin", "--password-stdin"],
            input=b"admin",
            env={**os.environ, "SEBAS_WEBUI_AUTH_DB": os.path.join(work, "auth.db")},
        )
        if result.returncode != 0:
            print("error: provisioning admin/admin failed", flush=True)
            raise SystemExit(1)

    log_path = os.path.join(work, "core.log")
    router_log_path = os.path.join(work, "router.log")
    log = open(log_path, "w")
    webui_log = None
    router_log = None
    core_proc = None
    router_proc = None
    if detached:
        # detached 双进程（core + 独立 webui，无 SEBAS_CORE_SECRET）本就无
        # router：core 旗标里已没有 --router，需要网关时另行手工拉起。
        webui_log_path = os.path.join(work, "webui.log")
        webui_log = open(webui_log_path, "w")
        env = _sandbox_env(work, secret=False)
        core_proc = subprocess.Popen(
            [sebas_bin, "core", "-c", os.path.join(work, "config.toml")],
            env=env,
            cwd=work,
            stdout=log,
            stderr=subprocess.STDOUT,
        )
        proc = subprocess.Popen(
            [sebas_bin, "webui", "-c", os.path.join(work, "config.toml")],
            env=env,
            cwd=work,
            stdout=webui_log,
            stderr=subprocess.STDOUT,
        )
        with open(os.path.join(work, "pids.json"), "w") as f:
            json.dump({"core": core_proc.pid, "webui": proc.pid}, f)
    else:
        # 两进程形态（unify-router-process-shape D5）：core（--webui，无
        # router 旗标）+ 独立 `sebas router --config … --debug` 子进程；
        # 清理侧对两个进程都 SIGTERM。
        cfg = os.path.join(work, "config.toml")
        env = _sandbox_env(work)
        router_log = open(router_log_path, "w")
        router_proc = subprocess.Popen(
            [sebas_bin, "router", "-c", cfg, "--debug"],
            env=env,
            cwd=work,
            stdout=router_log,
            stderr=subprocess.STDOUT,
        )
        proc = subprocess.Popen(
            [
                sebas_bin, "core", "-c", cfg,
                "--webui", "--webui-port", str(port),
            ],
            env=env,
            stdout=log,
            stderr=subprocess.STDOUT,
        )

    def _log_tail(n=30):
        tail = ""
        extra_logs = (
            [os.path.join(work, "webui.log")] if detached else [router_log_path]
        )
        for p in [log_path, *extra_logs]:
            if not p:
                continue
            try:
                with open(p) as f:
                    tail += "".join(f.readlines()[-n:])
            except OSError:
                pass
        return tail

    def _any_dead():
        if proc.poll() is not None:
            return True
        aux = core_proc if detached else router_proc
        return aux.poll() is not None

    # Readiness poll; a dead child during startup is an immediate error.
    for _ in range(120):
        if _health_ok(health_url):
            break
        if _any_dead():
            print("error: sebas process exited during startup; log:", flush=True)
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
        topology = (
            "detached 双进程（core + 独立 webui，无 SEBAS_CORE_SECRET）"
            if detached
            else "两进程（core --webui + 独立 router --debug）"
        )
        if auth_on:
            print(f"### webui 沙箱就绪：http://127.0.0.1:{port}/  （鉴权开启，admin / admin；{topology}）", flush=True)
        else:
            print(f"### webui 沙箱就绪：http://127.0.0.1:{port}/  （鉴权关闭，免登录；{topology}）", flush=True)
        print(f"### 日志与状态均在 {work}（core.log / router.log）；Ctrl-C 退出并清理", flush=True)

    while proc.poll() is None and not stop.is_set():
        stop.wait(0.5)

    # Teardown: SIGTERM the backend(s) — the router child too（两进程形态）,
    # including any core the Playwright fixture started (pids.json always
    # lists the CURRENT core pid) — then keep-or-clean (the reporter owns the
    # authoritative decision via `.tests-failed`; this is the fallback path).
    for child in ([proc, core_proc] if detached else [proc, router_proc]):
        if child is not None and child.poll() is None:
            child.terminate()
            try:
                child.wait(timeout=10)
            except subprocess.TimeoutExpired:
                child.kill()
    if detached:
        try:
            with open(os.path.join(work, "pids.json")) as f:
                recorded = [
                    pid for pid in json.load(f).values() if isinstance(pid, int)
                ]
        except (OSError, ValueError):
            recorded = []
        for pid in recorded:
            try:
                os.kill(pid, signal.SIGTERM)
            except (ProcessLookupError, PermissionError):
                pass
        # Graceful exit has a deadline; escalate to SIGKILL. Without this a
        # hung or late-started core outlives this process and its sandbox
        # dir — reparented to init, referenced by a deleted config (sebas-oo2).
        deadline = time.time() + 5
        alive = recorded
        while time.time() < deadline:
            alive = []
            for pid in recorded:
                try:
                    os.kill(pid, 0)
                    alive.append(pid)
                except (ProcessLookupError, PermissionError):
                    pass
            if not alive:
                break
            time.sleep(0.25)
        for pid in alive:
            try:
                os.kill(pid, signal.SIGKILL)
            except OSError:
                pass
    log.close()
    if webui_log is not None:
        webui_log.close()
    if router_log is not None:
        router_log.close()
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
        # The dir is gone, so any sebas still referencing it is by definition
        # an orphan (e.g. a fixture-restarted core that never made it into
        # pids.json) — the sweep is the last-resort reaper.
        _sweep_orphan_test_processes()


@task(
    help={
        "port": "webui port (default: 9879)",
        "auth": "enable auth with test account admin/admin",
        "keep": "keep the sandbox dir on exit",
    }
)
def testsuite_webui_sandbox(c, port=None, auth=False, keep=False):
    """Run a throwaway webui backend for GUI testing (foreground, Ctrl-C to stop)."""
    try:
        _run_webui_sandbox(
            port=int(port) if port else _TESTSUITE_HUMAN_PORT,
            auth_on=bool(auth),
            keep=bool(keep),
            reuse=bool(os.environ.get("TESTSUITE_REUSE")),
            human=True,
        )
    finally:
        _cleanup_stale_sandboxes()


@task
def testsuite_webui_server(c):
    """Blocking sandbox server for Playwright's webServer command (ports 9899/9898/9897).

    TESTSUITE_MODE=detached selects the dual-process topology (core without
    --webui + standalone `sebas webui`, NO SEBAS_CORE_SECRET — auto-arm and
    secret-file discovery, harden-core-channel-deployment 5.4); the default
    is the two-process `core --webui` + standalone `sebas router --debug`
    form (unify-router-process-shape)."""
    auth_on = os.environ.get("TESTSUITE_AUTH", "0") == "1"
    detached = os.environ.get("TESTSUITE_MODE", "") == "detached"
    if detached:
        default_port = 9897
    else:
        default_port = _TESTSUITE_AUTH_PORT if auth_on else _TESTSUITE_PORT
    try:
        _run_webui_sandbox(
            port=int(os.environ.get("TESTSUITE_PORT", default_port)),
            auth_on=auth_on,
            keep=os.environ.get("TESTSUITE_KEEP", "0") == "1",
            reuse=os.environ.get("TESTSUITE_REUSE", "0") == "1",
            human=os.environ.get("TESTSUITE_HUMAN", "0") == "1",
            detached=detached,
        )
    finally:
        _cleanup_stale_sandboxes()


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
