"""生成应用图标 assets/icon.ico（深底 + 三根指标柱）。

用法: python tools/make_icon.py
"""
from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "assets" / "icon.ico"
PNG = ROOT / "assets" / "icon-256.png"
PNG_BIG = ROOT / "assets" / "icon-1024.png"

S = 1024  # 先画大图再缩小，边缘更干净
BG = (14, 19, 28, 255)
EDGE = (44, 54, 70, 255)
BARS = [(59, 227, 196), (107, 160, 255), (140, 150, 168)]


def rounded(draw, box, r, fill):
    draw.rounded_rectangle(box, radius=r, fill=fill)


def main():
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)

    # 底板：圆角方块 + 一圈略亮的描边
    m = int(S * 0.045)
    rounded(d, (m, m, S - m, S - m), int(S * 0.22), BG)
    d.rounded_rectangle(
        (m, m, S - m, S - m), radius=int(S * 0.22), outline=EDGE, width=int(S * 0.012)
    )

    # 三根柱子：中间最高，底部对齐
    base_y = int(S * 0.79)
    w = int(S * 0.155)
    gap = int(S * 0.055)
    heights = [int(S * 0.30), int(S * 0.46), int(S * 0.22)]
    total = len(heights) * w + (len(heights) - 1) * gap
    x = (S - total) // 2
    for h, color in zip(heights, BARS):
        top = base_y - h
        d.rounded_rectangle(
            (x, top, x + w, base_y), radius=w // 2, fill=color + (255,)
        )
        x += w + gap

    OUT.parent.mkdir(parents=True, exist_ok=True)
    img.resize((256, 256), Image.LANCZOS).save(PNG, "PNG")
    # macOS 的 .icns 最多要 512@2x，留一张 1024 的原图给打包脚本切尺寸
    img.resize((1024, 1024), Image.LANCZOS).save(PNG_BIG, "PNG")
    img.save(
        OUT,
        format="ICO",
        sizes=[(16, 16), (20, 20), (24, 24), (32, 32), (40, 40), (48, 48), (64, 64), (128, 128), (256, 256)],
    )
    print("saved", OUT, OUT.stat().st_size, "bytes")


if __name__ == "__main__":
    main()
