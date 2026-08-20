#!/usr/bin/env python3

import os
import platform
import shutil
import shlex
import subprocess
import tempfile
import urllib.request
from pathlib import Path
import json

uname = platform.uname()
os_name = uname.system.lower()
machine = uname.machine.lower()

if os_name in {"macos", "osx"}:
    os_name = "darwin"
elif os_name in {"win"}:
    os_name = "windows"

is_windows = os_name == "windows"

os_name_machine = (os_name, machine)
print(f"os_name_machine: {os_name_machine}")


def target_triple() -> str:
    # Allows cross-compiling, e.g. building the x86_64 macOS binary on an
    # arm64 macOS runner, since Intel macOS runners are being retired.
    override = os.environ.get("CLI_BUILD_TARGET")
    if override:
        return override

    match os_name_machine:
        case ("darwin", "aarch64" | "arm64"):
            return "aarch64-apple-darwin"
        case ("darwin", "x86_64" | "amd64"):
            return "x86_64-apple-darwin"
        case ("linux", "aarch64" | "arm64"):
            return "aarch64-unknown-linux-musl"
        case ("linux", "x86_64"):
            return "x86_64-unknown-linux-musl"
        case ("windows", _):
            return "x86_64-pc-windows-msvc"
    raise SystemExit(f"Unsupported platform: {os_name_machine}")


def rustup_url() -> str:
    target = target_triple()
    if is_windows:
        return f"https://static.rust-lang.org/rustup/dist/{target}/rustup-init.exe"
    else:
        return f"https://static.rust-lang.org/rustup/dist/{target}/rustup-init"


def _run(args: list[str]) -> subprocess.CompletedProcess:
    print(f">> {shlex.join(args)}", flush=True)
    return subprocess.run(args)


def _run_check(args: list[str]):
    output = _run(args)
    print("stderr: ", output.stderr, flush=True)
    print("stdout: ", output.stdout, flush=True)
    output.check_returncode()


def _download_file(url: str, dest: Path):
    """Download a file, using PowerShell on Windows to avoid SSL cert issues."""
    if is_windows:
        _run_check(
            [
                "powershell",
                "-Command",
                f"Invoke-WebRequest -Uri '{url}' -OutFile '{dest}'",
            ]
        )
    else:
        urllib.request.urlretrieve(url, dest)


def _install_rustup():
    cargo = shutil.which("cargo")
    if cargo is not None:
        print(f"cargo already installed at {cargo}")
        return

    with tempfile.TemporaryDirectory() as td:
        url = rustup_url()
        name = url.split("/")[-1]
        rustup_bin = Path(td) / name

        print(f"Installing rustup from {url}")
        _download_file(url, rustup_bin)
        rustup_bin.chmod(0o755)

        _run([str(rustup_bin), "-y"])

    cargo = shutil.which("cargo")
    print(f"cargo installed at {cargo}")


def mkdir(path: Path) -> Path:
    path.mkdir(parents=True, exist_ok=True)
    return path


def with_os_ext(path: Path) -> Path:
    if is_windows and path.suffix.lower() != ".exe":
        return path.with_name(path.name + ".exe")
    return path


def main() -> None:
    home = Path.home()
    cwd = Path.cwd()

    # rustup and cargo live here
    cargo_bin = mkdir(home / ".cargo" / "bin")
    # needs to be shorter than default location so windows can build
    target_dir = mkdir(home / "target")
    # put the build artifacts here
    release_dir = mkdir(cwd / "release")

    os.environ["CARGO_TARGET_DIR"] = str(target_dir)
    os.environ["PATH"] = os.pathsep.join(
        [str(cargo_bin)]
        + os.environ.get("PATH", "").split(os.pathsep)
    )

    if os_name == "linux":
        # CI images run as root; hosted GitHub runners do not.
        sudo = [] if os.geteuid() == 0 else ["sudo"]
        _run_check(sudo + ["apt-get", "update"])
        _run_check(sudo + ["apt-get", "install", "-y", "musl-tools"])
        os.environ["CC_x86_64_unknown_linux_musl"] = "musl-gcc"

    _install_rustup()

    for bin in ["cargo", "rustup"]:
        print(f"{bin} installed at {shutil.which(bin)}")

    target = target_triple()
    _run_check(["rustup", "target", "add", target])
    _run_check(
        [
            "cargo",
            "build",
            "-p",
            "todo-curator",
            "--target",
            target,
            "--release",
        ]
    )

    exe = with_os_ext(target_dir / target / "release" / "todo-curator")
    name = f"todo-curator-{target}"
    exe = shutil.copy2(exe, with_os_ext(release_dir / name))
    exe = Path(exe)
    exe.chmod(0o755)

    print(f"todo-curator built at {exe}")


if __name__ == "__main__":
    main()
