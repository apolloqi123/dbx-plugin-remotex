#!/usr/bin/env python3
"""Container-file smoke for the container manager bottom pane.

Creates a throwaway local container and exercises the contract the workbench
uses for container files and live logs:

    list -> live log follow (2s) -> chunked upload -> import -> stat -> export
         -> saveLocal -> cleanup -> directory-download rejection

Two data planes are covered depending on the environment:

* sidecar plane (preferred): drives `docker/files/*` and `docker/logs/start|stop`
  over the stdio protocol with `target:"local"` (scripts/smoke/lib/sidecar_client.py).
* CLI plane (fallback when no sidecar binary is installed): runs the same
  contract directly through the docker CLI (`exec ls/stat`, `logs -f`,
  `cp` out/in, `rm`).

The script SKIPs (exit 0) only when the docker CLI, a reachable daemon, or a
local image with a POSIX shell is unavailable, so offline CI stays green. A
missing sidecar binary is *not* a SKIP: the run falls back to the CLI plane and
still reports PASS. Only a throwaway container and temp directories are created;
both are removed on exit (unless --keep), and no existing container/image is
modified.

Usage:
    python3 scripts/smoke/smoke_docker_files.py [--binary path/to/dbx-plugin-sshx]
                                                [--keep] [--timeout 120]
"""

from __future__ import annotations

import argparse
import base64
import json
import os
import shutil
import struct
import subprocess
import sys
import tempfile
import time
import uuid

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib"))

SKIP_EXIT = 0
FAIL_EXIT = 1
CONTAINER_PREFIX = "dbx-sshx-smoke-"
SMOKE_FILE = "dbx-sshx-smoke.txt"
SMOKE_BODY = b"dbx-sshx container file smoke\n"


def log(message: str) -> None:
    print(f"  {message}", flush=True)


def skip(message: str) -> int:
    print(f"SKIP: {message}")
    return SKIP_EXIT


def run(argv: list[str], timeout: float = 60.0, check: bool = True) -> subprocess.CompletedProcess:
    result = subprocess.run(argv, capture_output=True, text=True, timeout=timeout)
    if check and result.returncode != 0:
        raise AssertionError(
            f"{' '.join(argv)} failed (exit {result.returncode}): "
            f"{(result.stderr or result.stdout).strip()}"
        )
    return result


def docker_info(cli: str) -> bool:
    try:
        return run([cli, "info"], timeout=20, check=False).returncode == 0
    except (OSError, subprocess.TimeoutExpired):
        return False


def candidate_images(cli: str) -> list[str]:
    """Local images only (never pull in a smoke); sh-capable names first."""
    result = run([cli, "images", "--format", "{{.Repository}}:{{.Tag}}"], check=False)
    images = [
        line.strip()
        for line in result.stdout.splitlines()
        if line.strip() and "<none>" not in line
    ]
    preferred = ("alpine", "busybox", "debian", "ubuntu", "python", "nginx", "redis")
    images.sort(key=lambda name: next(
        (index for index, token in enumerate(preferred) if token in name), len(preferred)
    ))
    return images[:6]


def start_container(cli: str, image: str, name: str, timeout: float) -> bool:
    """`echo` loop keeps stdout producing lines so `logs -f` really streams."""
    try:
        result = run(
            [
                cli, "run", "-d", "--name", name, "--entrypoint", "sh", image,
                "-c", "while true; do echo dbx-heartbeat; sleep 1; done",
            ],
            timeout=timeout,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired):
        return False
    if result.returncode != 0:
        run([cli, "rm", "-f", name], timeout=30, check=False)
        log(f"image {image} cannot run a POSIX shell, trying next")
        return False
    return True


def container_id(cli: str, name: str) -> str:
    return run([cli, "inspect", "-f", "{{.Id}}", name], timeout=20).stdout.strip()


def has_file_tools(cli: str, name: str) -> bool:
    """Browsing needs `ls`; entry details need `stat` (distroless images lack both)."""
    result = run(
        [cli, "exec", name, "sh", "-c", "command -v ls >/dev/null && command -v stat >/dev/null"],
        timeout=30,
        check=False,
    )
    return result.returncode == 0


def remove_container(cli: str, name: str) -> None:
    run([cli, "rm", "-f", name], timeout=60, check=False)


def sidecar_payload(result: dict) -> dict:
    """Workbench invokes return {content:[{text}]} on some builds."""
    if isinstance(result, dict) and "content" in result and "output" not in result:
        return json.loads(result["content"][0]["text"])
    return result


class SidecarPlane:
    """`docker/files/*` + `docker/logs/*` over the real sidecar."""

    def __init__(self, binary: str, workdir: str):
        from sidecar_client import SidecarClient  # imported lazily: optional plane

        os.makedirs(os.path.join(workdir, "data"), exist_ok=True)
        os.makedirs(os.path.join(workdir, "downloads"), exist_ok=True)
        previous = os.environ.get("DBX_SSH_DOWNLOAD_DIR")
        os.environ["DBX_SSH_DOWNLOAD_DIR"] = os.path.join(workdir, "downloads")
        try:
            self.client = SidecarClient.start(
                binary=binary, data_dir=os.path.join(workdir, "data"), timeout=40
            )
            self.client.initialize()
        finally:
            if previous is None:
                os.environ.pop("DBX_SSH_DOWNLOAD_DIR", None)
            else:
                os.environ["DBX_SSH_DOWNLOAD_DIR"] = previous
        self.downloads = os.path.join(workdir, "downloads")

    def request(self, method: str, params: dict, timeout: float = 60) -> dict:
        return sidecar_payload(self.client.request(method, params, timeout=timeout))

    def close(self) -> None:
        self.client.close()


def sidecar_smoke(plane: SidecarPlane, cid: str, timeout: float) -> None:
    listed = plane.request("docker/list", {"target": "local"}, timeout=40)
    assert isinstance(listed.get("containers"), list), listed
    assert any(cid.startswith(row.get("id", "")) or row.get("id", "").startswith(cid[:12])
               for row in listed["containers"]), "temp container missing from docker/list"
    log("docker/list target=local ok")

    # 1) live follow: start, wait for a heartbeat chunk, stop.
    started = plane.request(
        "docker/logs/start", {"target": "local", "containerId": cid, "tail": 50}, timeout=30
    )
    stream_id = started.get("streamId")
    assert isinstance(stream_id, str) and stream_id, started
    chunk = plane.client.wait_event("docker/logs/chunk", timeout=min(10, timeout))
    assert chunk is not None, "no docker/logs/chunk event within timeout"
    params = chunk.get("params", {})
    assert params.get("streamId") == stream_id, params
    assert isinstance(params.get("text"), str), params
    stopped = plane.request("docker/logs/stop", {"streamId": stream_id}, timeout=20)
    assert stopped.get("stopped") is True, stopped
    # A manual stop must close the stream with the same cancelled end event the
    # workbench state machine consumes (PROTOCOL: error:"cancelled").
    ended = plane.client.wait_event("docker/logs/end", timeout=min(10, timeout))
    assert ended is not None, "no docker/logs/end after stop"
    end_params = ended.get("params", {})
    assert end_params.get("streamId") == stream_id, end_params
    assert end_params.get("error") == "cancelled", end_params
    log(f"docker/logs/start|stop ok (streamId={stream_id}, chunk={params['text'].strip()!r})")

    # 2) PTY terminal: open a local container shell, resize it, type a command
    # over the binary input channel and verify both the output and exit state.
    terminal_id = str(uuid.uuid4())
    opened = plane.request(
        "docker/terminal/open",
        {"target": "local", "containerId": cid, "terminalId": terminal_id, "cols": 80, "rows": 24},
        timeout=40,
    )
    assert opened.get("terminalId") == terminal_id, opened
    resized = plane.request(
        "docker/terminal/resize", {"terminalId": terminal_id, "cols": 100, "rows": 30}, timeout=20
    )
    assert resized.get("resized") is True, resized
    plane.client.send_binary(
        f"docker/terminal/in/{terminal_id}",
        struct.pack(">Q", 1) + b"echo DBX_PTY_OK; exit 7\r",
    )
    state = plane.client.wait_event("docker/terminal/state", timeout=min(20, timeout))
    assert state is not None, "no docker/terminal/state after the shell exited"
    state_params = state.get("params", {})
    assert state_params.get("terminalId") == terminal_id, state_params
    assert state_params.get("state") == "exited", state_params
    assert state_params.get("exitCode") == 7, state_params
    output = b"".join(
        data
        for channel, data in plane.client.binary_frames
        if channel == f"docker/terminal/out/{terminal_id}"
    )
    assert b"DBX_PTY_OK" in output, f"shell output missing marker: {output[:200]!r}"
    assert plane.request("docker/terminal/close", {"terminalId": terminal_id}, timeout=20).get(
        "stopped"
    ) is False, "an exited terminal must not be reported as actively stopped"
    log(f"docker/terminal open/input/resize/exit ok (terminalId={terminal_id})")

    # 3) browse: root listing + stat of a known file.
    root = plane.request("docker/files/list", {"target": "local", "containerId": cid, "path": "/"})
    assert root.get("path") == "/", root
    assert isinstance(root.get("entries"), list) and root["entries"], root
    for entry in root["entries"]:
        assert {"name", "path", "kind"} <= set(entry), entry
    nested = plane.request(
        "docker/files/list", {"target": "local", "containerId": cid, "path": "/tmp"}
    )
    assert nested.get("path") == "/tmp", nested
    log(f"docker/files/list ok ({len(root['entries'])} root entries)")

    # 3) chunked upload -> import -> stat -> export -> saveLocal -> cleanup.
    begun = plane.request("docker/files/upload/begin", {"target": "local", "name": SMOKE_FILE})
    upload_id = begun.get("uploadId")
    assert upload_id, begun
    received = plane.request(
        "docker/files/upload/chunk",
        {
            "target": "local",
            "uploadId": upload_id,
            "offset": 0,
            "dataBase64": base64.b64encode(SMOKE_BODY).decode(),
        },
    )
    assert received.get("received") == len(SMOKE_BODY), received
    finished = plane.request(
        "docker/files/upload/finish", {"target": "local", "uploadId": upload_id}
    )
    staging = finished.get("stagingPath")
    assert staging and finished.get("size") == len(SMOKE_BODY), finished

    imported = plane.request(
        "docker/files/import",
        {"target": "local", "containerId": cid, "stagingPath": staging, "destPath": "/tmp/"},
    )
    dest = imported.get("path", "")
    assert dest == f"/tmp/{SMOKE_FILE}", imported
    stat = plane.request(
        "docker/files/stat", {"target": "local", "containerId": cid, "path": dest}
    )
    assert stat.get("kind") == "file" and stat.get("size") == len(SMOKE_BODY), stat
    log(f"upload/import ok ({dest}, {stat['size']} bytes)")

    exported = plane.request(
        "docker/files/export", {"target": "local", "containerId": cid, "path": dest}
    )
    assert exported.get("location") == "local", exported
    assert exported.get("size") == len(SMOKE_BODY), exported
    saved = plane.request(
        "docker/files/saveLocal",
        {
            "target": "local",
            "stagingPath": exported["stagingPath"],
            "name": exported["name"],
            "conflict": "overwrite",
        },
    )
    local_path = saved.get("localPath", "")
    assert local_path and os.path.exists(local_path), saved
    with open(local_path, "rb") as handle:
        assert handle.read() == SMOKE_BODY, "saved content mismatch"
    plane.request(
        "docker/files/cleanup",
        {"target": "local", "stagingPath": exported["stagingPath"]},
    )
    cleanup = plane.request(
        "docker/files/cleanup", {"target": "local", "stagingPath": staging}
    )
    assert cleanup.get("removed") is True, cleanup
    assert not os.path.exists(staging), "upload staging file survived cleanup"
    log(f"export/saveLocal/cleanup ok ({local_path})")

    # 4) v1 limit: directories are never downloadable.
    try:
        plane.request("docker/files/export", {"target": "local", "containerId": cid, "path": "/tmp"})
        raise AssertionError("docker/files/export accepted a directory")
    except AssertionError:
        raise
    except Exception as error:  # noqa: BLE001 - sidecar errors surface as exceptions
        assert "目录暂不支持下载" in str(error) or "directory" in str(error).lower(), error
    log("directory download rejected ok")


def cli_smoke(cli: str, cid: str, workdir: str, timeout: float) -> None:
    """Direct docker CLI contract, used when no sidecar binary is installed."""
    root = run([cli, "exec", cid, "ls", "-1ap", "--", "/"], timeout=30).stdout
    entries = [line for line in root.splitlines() if line]
    assert "tmp/" in entries or "etc/" in entries, root
    log(f"exec ls -1ap ok ({len(entries)} entries)")

    body = SMOKE_BODY.decode().strip()
    run([cli, "exec", cid, "sh", "-c", f"printf '%s\\n' '{body}' > /tmp/{SMOKE_FILE}"], timeout=30)
    stat = run(
        [cli, "exec", cid, "stat", "-c", "%F|%s|%Y|%a", "--", f"/tmp/{SMOKE_FILE}"],
        timeout=30,
    ).stdout.strip()
    file_type, size, _mtime, mode = stat.split("|")
    assert file_type in ("regular file", "regular empty file"), stat
    assert int(size) == len(SMOKE_BODY), stat
    log(f"exec stat ok ({file_type}, {size} bytes, mode {mode})")

    # logs -f: follow for two seconds, then terminate the follower.
    follower = subprocess.Popen(
        [cli, "logs", "-f", "--tail", "5", cid],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    time.sleep(2)
    follower.terminate()
    try:
        followed, _ = follower.communicate(timeout=10)
    except subprocess.TimeoutExpired:
        follower.kill()
        followed, _ = follower.communicate(timeout=10)
    assert "dbx-heartbeat" in followed, f"logs -f produced no heartbeat: {followed!r}"
    log("logs -f followed for 2s ok")

    staging_dir = os.path.join(workdir, "cli-staging")
    os.makedirs(staging_dir, exist_ok=True)
    exported = os.path.join(staging_dir, SMOKE_FILE)
    run([cli, "cp", f"{cid}:/tmp/{SMOKE_FILE}", exported], timeout=60)
    with open(exported, "rb") as handle:
        assert handle.read() == SMOKE_BODY, "docker cp export content mismatch"
    imported_name = "dbx-sshx-imported.txt"
    run([cli, "cp", exported, f"{cid}:/tmp/{imported_name}"], timeout=60)
    roundtrip = run([cli, "exec", cid, "cat", f"/tmp/{imported_name}"], timeout=30).stdout
    assert roundtrip.strip() == body, roundtrip
    log("docker cp export/import roundtrip ok")

    run([cli, "exec", cid, "rm", "-f", f"/tmp/{SMOKE_FILE}", f"/tmp/{imported_name}"], timeout=30)
    os.remove(exported)
    missing = run(
        [cli, "exec", cid, "ls", "-1a", "--", f"/tmp/{SMOKE_FILE}"], timeout=30, check=False
    )
    assert missing.returncode != 0, "container file survived cleanup"
    log("container + host staging cleanup ok")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", default=None, help="sidecar binary path")
    parser.add_argument("--keep", action="store_true", help="keep the temp container")
    parser.add_argument("--timeout", type=float, default=120.0, help="per-command timeout")
    args = parser.parse_args()

    cli = shutil.which("docker")
    if not cli:
        return skip("docker CLI not found on PATH")
    if not docker_info(cli):
        return skip("docker daemon is not reachable")

    images = candidate_images(cli)
    if not images:
        return skip("no local docker images to run a temp container from")

    workdir = tempfile.mkdtemp(prefix="dbx-sshx-smoke-")
    name = f"{CONTAINER_PREFIX}{uuid.uuid4().hex[:8]}"
    started = False
    try:
        for image in images:
            log(f"starting temp container from {image}")
            if start_container(cli, image, name, args.timeout):
                if not has_file_tools(cli, name):
                    log(f"image {image} lacks ls/stat (distroless), trying next")
                    remove_container(cli, name)
                    continue
                started = True
                log(f"container {name} from {image} ok")
                break
        if not started:
            return skip("no local image could run `sh` with a heartbeat loop")

        cid = container_id(cli, name)
        assert len(cid) >= 12, cid

        sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib"))
        from sidecar_client import default_binary  # noqa: E402

        binary = args.binary or os.environ.get("DBX_PLUGIN_SIDECAR") or default_binary()
        if binary and os.path.exists(binary):
            plane = SidecarPlane(binary, workdir)
            try:
                sidecar_smoke(plane, cid, args.timeout)
            finally:
                plane.close()
            print(f"PASS: container file smoke (sidecar plane, image={image}, id={cid[:12]})")
        else:
            log("sidecar binary not found — exercising the docker CLI contract instead")
            cli_smoke(cli, cid, workdir, args.timeout)
            print(f"PASS: container file smoke (docker CLI plane, image={image}, id={cid[:12]})")
        return 0
    except AssertionError as error:
        print(f"FAIL: {error}")
        return FAIL_EXIT
    except Exception as error:  # noqa: BLE001 - sidecar/CLI errors are smoke failures
        print(f"FAIL: {type(error).__name__}: {error}")
        return FAIL_EXIT
    finally:
        if started and not args.keep:
            remove_container(cli, name)
        if args.keep and started:
            log(f"keeping container {name}")
        shutil.rmtree(workdir, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
