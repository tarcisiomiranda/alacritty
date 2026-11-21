import math
import subprocess
import struct
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ASSETS = ROOT / "extra/logo"


def gear():
    points = []
    for tooth in range(12):
        for offset, radius in [(-15, 98), (-10, 98), (-7, 116), (7, 116), (10, 98)]:
            angle = math.radians(tooth * 30 + offset)
            points.append(f"{128 + radius * math.cos(angle):.3f},{128 + radius * math.sin(angle):.3f}")
    return "M" + " L".join(points) + " Z"


def mark():
    return (
        f'<path d="{gear()}" fill="#e87842" stroke="#e87842" stroke-width="5" stroke-linejoin="round"/>'
        '<circle cx="128" cy="128" r="76" fill="#20262e"/>'
        '<path d="M91 105 L115 128 L91 151 M135 151 H165" fill="none" stroke="#fff4e9" '
        'stroke-width="11" stroke-linecap="round" stroke-linejoin="round"/>'
    )


def svg(width, height, body, title):
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" '
        f'viewBox="0 0 {width} {height}" role="img" aria-label="{title}">'
        f'<title>{title}</title>{body}</svg>\n'
    )


def render(source, destination, size):
    subprocess.run([
        "magick", "-background", "none", "-density", "384", str(source),
        "-resize", size, "-depth", "8", "-strip", f"PNG32:{destination}",
    ], check=True)


def main():
    icon = ASSETS / "termust.svg"
    icon.write_text(svg(256, 256, mark(), "Termust gear and terminal logo"))
    render(icon, ASSETS / "termust.png", "256x256")
    for name, foreground in [("termust-wordmark", "#20262e"), ("termust-wordmark-light", "#fff4e9")]:
        body = f'<g transform="translate(0 8) scale(.75)">{mark()}</g>'
        body += (
            f'<text x="214" y="137" fill="{foreground}" font-family="DejaVu Sans, sans-serif" '
            'font-size="86" font-weight="700" letter-spacing="-4">Termust</text>'
        )
        source = ASSETS / f"{name}.svg"
        source.write_text(svg(640, 208, body, "Termust"))
        render(source, ASSETS / f"{name}.png", "1280x416")
    body = '<rect width="1080" height="640" rx="24" fill="#13181e"/>'
    body += '<text x="52" y="58" fill="#d8dce1" font-family="DejaVu Sans, sans-serif" font-size="18">TERMUST / VISUAL IDENTITY</text>'
    body += f'<g transform="translate(52 100)">{mark()}</g>'
    body += '<text x="353" y="233" fill="#fff4e9" font-family="DejaVu Sans, sans-serif" font-size="100" font-weight="700" letter-spacing="-5">Termust</text>'
    body += '<text x="358" y="281" fill="#9ba6b2" font-family="DejaVu Sans, sans-serif" font-size="22">A Rust terminal, built for your workflow.</text>'
    body += '<rect x="36" y="404" width="1008" height="200" rx="18" fill="#f5efe9"/>'
    for x, size in [(72, 128), (252, 64), (360, 48), (448, 32), (520, 24)]:
        body += f'<g transform="translate({x} {436 + (128 - size) / 2}) scale({size / 256})">{mark()}</g>'
    body += '<text x="670" y="476" fill="#20262e" font-family="DejaVu Sans, sans-serif" font-size="17">COPPER</text>'
    body += '<rect x="670" y="492" width="64" height="38" rx="6" fill="#e87842"/>'
    body += '<text x="750" y="517" fill="#20262e" font-family="DejaVu Sans, sans-serif" font-size="20">#E87842</text>'
    source = ASSETS / "termust-preview.svg"
    source.write_text(svg(1080, 640, body, "Termust logo on dark and light backgrounds"))
    render(source, ASSETS / "termust-preview.png", "1080x640")
    subprocess.run([
        "magick", str(ASSETS / "termust.png"), "-define", "icon:auto-resize=256,128,64,48,32,16",
        str(ASSETS / "termust.ico"),
    ], check=True)
    chunks = []
    with tempfile.TemporaryDirectory(prefix="termust-icons-") as temporary:
        for kind, size in [(b"icp4", 16), (b"icp5", 32), (b"icp6", 64), (b"ic07", 128),
                           (b"ic08", 256), (b"ic09", 512), (b"ic10", 1024)]:
            target = Path(temporary) / f"{size}.png"
            render(icon, target, f"{size}x{size}")
            data = target.read_bytes()
            chunks.append(kind + struct.pack(">I", len(data) + 8) + data)
    content = b"".join(chunks)
    (ASSETS / "termust.icns").write_bytes(b"icns" + struct.pack(">I", len(content) + 8) + content)
    bundle = ROOT / "extra/osx/Termust.app/Contents/Resources/termust.icns"
    bundle.parent.mkdir(parents=True, exist_ok=True)
    bundle.write_bytes((ASSETS / "termust.icns").read_bytes())
    print("Generated Termust SVG, PNG, ICO and ICNS assets.")


if __name__ == "__main__":
    main()
