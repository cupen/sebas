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
