"""Maintainer tool: package an already-built Windows release and its notices."""
import hashlib
import json
from pathlib import Path
import subprocess
import tomllib
import zipfile

ROOT = Path(__file__).resolve().parents[1]


def command(*args):
    return subprocess.check_output(args, cwd=ROOT, text=True, encoding="utf-8")


def main():
    if command("git", "status", "--porcelain").strip():
        raise SystemExit("Commit source changes before packaging a release.")
    version = tomllib.loads((ROOT / "desktop/Cargo.toml").read_text())["package"]["version"]
    metadata = json.loads(command(
        "cargo", "metadata", "--manifest-path", "desktop/Cargo.toml", "--locked",
        "--format-version", "1", "--filter-platform", "x86_64-pc-windows-msvc",
    ))
    output = ROOT / "outputs/release" / f"v{version}"
    output.mkdir(parents=True, exist_ok=True)
    archive = output / f"LocalVoice-v{version}-windows-x64.zip"
    notices = ["# Third-party components\n",
               "Slint is used under the Slint Royalty-free Desktop, Mobile, and Web Applications License 2.0.",
               "The download page displays the Made with Slint attribution badge.",
               "License files below cover the resolved Cargo dependency set, including build dependencies.\n"]
    # Cargo archives often retain timestamps older than ZIP's 1980 minimum.
    with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED, strict_timestamps=False) as bundle:
        bundle.write(ROOT / "desktop/target/release/local-voice-desktop.exe", "local-voice-desktop.exe")
        bundle.writestr("build-info.json", json.dumps({
            "version": version,
            "commit": command("git", "rev-parse", "HEAD").strip(),
            "target": "x86_64-pc-windows-msvc",
            "rustc": command("rustc", "--version").strip(),
        }, indent=2) + "\n")
        bundle.write(ROOT / "install.md", "install.md")
        for package in sorted(metadata["packages"], key=lambda p: (p["name"], p["version"])):
            if package["source"] is None:
                continue
            name = f'{package["name"]}-{package["version"]}'
            directory = Path(package["manifest_path"]).parent
            entries = [p for p in directory.iterdir() if p.name.lower().startswith(
                ("license", "licence", "copying", "notice", "copyright"))]
            if not entries:
                directory = ROOT / "desktop/third-party" / name
                entries = list(directory.iterdir())  # Missing notices must fail packaging.
            for entry in entries:
                files = [entry] if entry.is_file() else [p for p in entry.rglob("*") if p.is_file()]
                for path in files:
                    bundle.write(path, f"licenses/{name}/{path.relative_to(directory).as_posix()}")
            notices.append(f'- {name}: {package["license"]}; {package.get("repository") or package.get("homepage") or "see crate metadata"}')
        bundle.writestr("THIRD-PARTY-NOTICES.md", "\n".join(notices) + "\n")
    with archive.open("rb") as source:
        digest = hashlib.file_digest(source, "sha256").hexdigest()
    (output / "SHA256SUMS").write_text(f"{digest}  {archive.name}\n", encoding="ascii")
    print(archive)
    print(digest)


if __name__ == "__main__":
    main()
