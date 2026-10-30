"""Generate native icon assets from the application's existing waveform mark.

Requires Pillow; generated assets are checked in, so builds do not need Python.
The geometry and colors match assets/app.svg and the sidebar mark.
"""
from pathlib import Path
from PIL import Image, ImageDraw

root = Path(__file__).resolve().parent.parent / "assets"
root.mkdir(exist_ok=True)
size = 1152
scale = size / 36
master = Image.new("RGBA", (size, size))
draw = ImageDraw.Draw(master)
draw.rounded_rectangle((0, 0, size - 1, size - 1), radius=10 * scale, fill="#5b4ee8")
for i, height in enumerate((9, 19, 13, 23, 11)):
    x = (9 + i * 4.5) * scale
    draw.rectangle((x - scale, (18 - height / 2) * scale,
                    x + scale, (18 + height / 2) * scale), fill="white")
master.resize((256, 256), Image.Resampling.LANCZOS).save(
    root / "app.ico", sizes=[(n, n) for n in (16, 20, 24, 32, 40, 48, 64, 128, 256)])
(root / "app-128.rgba").write_bytes(master.resize((128, 128), Image.Resampling.LANCZOS).tobytes())
print("Generated multi-resolution EXE icon and 128px RGBA window icon.")
