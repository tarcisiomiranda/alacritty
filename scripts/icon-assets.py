import subprocess
from pathlib import Path


root = Path(__file__).resolve().parents[1]
for name in ("cat", "dog", "fox", "robot"):
    source = root / "extra/icons" / f"{name}.svg"
    subprocess.run(["magick", "-background", "none", str(source),
        "-define", "png:color-type=6", str(source.with_suffix(".png"))], check=True)
for name in ("cat", "dog", "fox", "robot", "yorkie", "termust"):
    source = root / ("extra/logo" if name == "termust" else "extra/icons") / f"{name}.png"
    target = root / "extra/icons" / f"{name}.ico"
    subprocess.run(["magick", str(source), "-define", "icon:auto-resize=256,64,48,32,16",
        str(target)], check=True)
