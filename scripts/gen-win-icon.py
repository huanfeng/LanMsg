#!/usr/bin/env python3
"""生成 Windows exe 图标 crates/lanmsg-desktop/assets/lanmsg.ico（依赖 Pillow）。

造型与 assets/icons/logo.svg 一致：深青圆角方块 + 白色对话气泡（28x28 视口）。
用法：python3 scripts/gen-win-icon.py
"""
from pathlib import Path
from PIL import Image, ImageDraw

SIZES = [16, 24, 32, 48, 64, 256]
SS = 2048  # 超采样画布边长
K = SS / 28  # 视口坐标 -> 像素

def p(x, y):
    return (x * K, y * K)

def render():
    img = Image.new("RGBA", (SS, SS), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    d.rounded_rectangle([0, 0, SS - 1, SS - 1], radius=7 * K, fill=(0x0B, 0x7A, 0x75, 255))
    white = (255, 255, 255, 255)
    # 气泡主体：x 7.5..20.5, y 8..17.5, 圆角 2
    d.rounded_rectangle([7.5 * K, 8 * K, 20.5 * K, 17.5 * K], radius=2 * K, fill=white)
    # 气泡尾巴
    d.polygon([p(13.25, 17.5), p(10, 20.5), p(10, 17.5)], fill=white)
    return img

def main():
    out = Path(__file__).resolve().parent.parent / "crates/lanmsg-desktop/assets/lanmsg.ico"
    render().save(out, format="ICO", sizes=[(s, s) for s in SIZES])
    print(f"wrote {out}")

if __name__ == "__main__":
    main()
