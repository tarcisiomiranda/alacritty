import configparser
import json
import struct
import tomllib
import xml.etree.ElementTree as ET
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ASSETS = ROOT / "extra/logo"


def main():
    icon = ASSETS / "termust.png"
    data = icon.read_bytes()
    assert data[:8] == b"\x89PNG\r\n\x1a\n"
    width, height, depth, color = struct.unpack(">IIBB", data[16:26])
    assert (width, height, depth, color) == (256, 256, 8, 6), "Window icon must be RGBA8"
    for name in ["termust", "termust-wordmark", "termust-wordmark-light", "termust-preview"]:
        tree = ET.parse(ASSETS / f"{name}.svg")
        assert tree.getroot().tag == "{http://www.w3.org/2000/svg}svg"
        assert tree.getroot().find("{http://www.w3.org/2000/svg}title") is not None
        assert (ASSETS / f"{name}.png").stat().st_size > 1000
    ico = (ASSETS / "termust.ico").read_bytes()
    assert struct.unpack("<HHH", ico[:6]) == (0, 1, 6)
    icns = (ASSETS / "termust.icns").read_bytes()
    assert icns[:4] == b"icns" and struct.unpack(">I", icns[4:8])[0] == len(icns)
    offset = 8
    entries = 0
    while offset < len(icns):
        length = struct.unpack(">I", icns[offset + 4:offset + 8])[0]
        assert length > 8 and offset + length <= len(icns)
        assert icns[offset + 8:offset + 16] == b"\x89PNG\r\n\x1a\n"
        offset += length
        entries += 1
    assert entries == 7 and offset == len(icns)
    desktop = configparser.ConfigParser(interpolation=None, strict=False)
    desktop.read(ROOT / "extra/linux/Termust.desktop")
    entry = desktop["Desktop Entry"]
    assert (entry["Name"], entry["Exec"], entry["Icon"], entry["StartupWMClass"]) == (
        "Termust", "termust", "Termust", "Termust")
    assert entry["Actions"] == "new-window;"
    assert desktop["Desktop Action new-window"]["Exec"] == "termust"
    manifest = tomllib.loads((ROOT / "alacritty/Cargo.toml").read_text())
    assert manifest["package"]["name"] == "termust"
    assert "src=\"extra/logo/termust.svg\"" in (ROOT / "README.md").read_text()
    assert "TARGET = termust" in (ROOT / "Makefile").read_text()
    app_icon = ROOT / "extra/osx/Termust.app/Contents/Resources/termust.icns"
    assert app_icon.read_bytes() == icns
    for name in ["termust.bash", "termust.fish", "_termust"]:
        assert (ROOT / "extra/completions" / name).stat().st_size > 1000
    print(json.dumps({"identity": "Termust", "window_icon": [width, height, "RGBA8"],
        "vector_variants": 4, "windows_icon_sizes": 6, "mac_icon_sizes": entries,
        "desktop": "Termust.desktop", "result": "verified"}))


if __name__ == "__main__":
    main()
