"""Build a self-contained Linux directory with its runtime assets."""

from pathlib import Path
import subprocess
import sys


def run_pyinstaller_build():
    root = Path(__file__).resolve().parent
    command = [
        sys.executable, "-m", "PyInstaller",
        "--onedir", "--contents-directory", ".",
        "--name", "Writing Tools", "--clean", "--noconfirm",
        "--paths", str(root / "src"),
        "--exclude-module", "tkinter",
        "--exclude-module", "IPython",
        "--exclude-module", "unittest",
    ]
    # Keep assets next to the executable: the UI resolves them there and
    # options.json is editable. Never include the user's config.json.
    editable = ("options.json", "options_examples.json")
    bundled = (
        "icons", "locales",
        "background.png", "background_dark.png",
        "background_popup.png", "background_popup_dark.png",
    )
    sources = [root / name for name in editable] + [root / "assets" / name for name in bundled]
    for source in sources:
        destination = source.name if source.is_dir() else "."
        command.extend(["--add-data", f"{source}:{destination}"])
    command.append(str(root / "main.py"))
    subprocess.run(command, cwd=root, check=True)
    print(f"Build completed: {root / 'dist' / 'Writing Tools'}")


if __name__ == "__main__":
    run_pyinstaller_build()
