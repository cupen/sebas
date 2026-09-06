"""Invoke tasks for sebas.

Usage:
    invoke build-image              # build image
    invoke build-image --push       # build + push to ghcr.io
    invoke build-image --tag v0.1.0 # tag with a version
    invoke build-image --no-cache   # force rebuild
    invoke --help                   # list all tasks
"""

import os
import subprocess
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

@task(help={"case": "Run a single e2e case by name (cargo test filter)"})
def e2e(c, case=None):
    """Build the workspace and run the process-level core-flow e2e suite."""
    print("Building workspace (sebas + fake-claude) ...")
    result = c.run("cargo build", echo=True)
    if result.failed:
        raise SystemExit(1)
    test_filter = f"{case} " if case else ""
    cmd = f"cargo test --test core_flow_e2e_test {test_filter}-- --ignored".replace("  ", " ")
    result = c.run(cmd, echo=True)
    if result.failed:
        print("e2e suite FAILED; kept sandbox dirs are printed above (or under target/tests/)")
        raise SystemExit(1)


@task(help={"case": "Run a single acceptance journey by name (cargo test filter)"})
def accept(c, case=None):
    """Build the workspace and run the acceptance suite (journey-level)."""
    print("Building workspace (sebas + fake-claude) ...")
    result = c.run("cargo build", echo=True)
    if result.failed:
        raise SystemExit(1)
    test_filter = f"{case} " if case else ""
    cmd = f"cargo test --test acceptance_suite_test {test_filter}-- --ignored".replace("  ", " ")
    result = c.run(cmd, echo=True)
    if result.failed:
        print("acceptance suite FAILED; kept sandbox dirs are printed above (or under target/tests/)")
        raise SystemExit(1)


def _webui_e2e_preflight(c):
    """pnpm + Playwright chromium preflight; installs what is missing."""
    result = c.run("pnpm --version", hide=True, warn=True)
    if result.failed:
        print("❌ pnpm is required (corepack enable or install pnpm)")
        raise SystemExit(1)
    e2e_dir = "tests/webui-e2e"
    if not os.path.isdir(os.path.join(e2e_dir, "node_modules")):
        print("Installing tests/webui-e2e dependencies ...")
        if c.run(f"pnpm install --dir {e2e_dir}", echo=True).failed:
            raise SystemExit(1)
    result = c.run(
        f"pnpm --dir {e2e_dir} exec playwright install chromium", echo=True, warn=True
    )
    # Already-installed browsers exit 0; a failed download blocks the run.
    if result.failed and "already installed" not in (result.stdout or ""):
        raise SystemExit(1)


@task(help={"case": "Run a single webui journey (spec file stem: first-paint, auth, ...)"})
def webui_e2e(c, case=None):
    """Build + run the browser-level webui e2e suite (Playwright, sandboxed)."""
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

    _webui_e2e_preflight(c)

    e2e_dir = "tests/webui-e2e"
    if case:
        # --case auth runs the auth-on form; anything else filters the main suite.
        if case == "auth":
            cmd = f"pnpm --dir {e2e_dir} exec playwright test --config playwright.auth.config.ts"
        else:
            cmd = f"pnpm --dir {e2e_dir} exec playwright test {case}"
    else:
        cmd = f"pnpm --dir {e2e_dir} exec playwright test && pnpm --dir {e2e_dir} exec playwright test --config playwright.auth.config.ts"
    result = c.run(cmd, echo=True)
    if result.failed:
        print(
            "webui-e2e FAILED — sandbox scene kept for debugging (path printed above)."
        )
        print("Reuse it interactively: E2E_REUSE=1 E2E_KEEP=1 invoke webui-e2e --case <name>")
        raise SystemExit(1)


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
