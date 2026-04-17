#!/usr/bin/env python3
"""将 scripts/image 目录下的图片统一缩放到 240x240 分辨率（覆盖原图）。"""

import os
import sys

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
IMAGE_DIR = os.path.join(SCRIPT_DIR, "image")
TARGET_SIZE = (240, 240)
EXT = (".png", ".jpg", ".jpeg", ".gif", ".bmp", ".webp")


def main():
    try:
        from PIL import Image
    except ImportError:
        print("需要安装 Pillow: pip install Pillow", file=sys.stderr)
        sys.exit(1)
    if not os.path.isdir(IMAGE_DIR):
        print(f"目录不存在: {IMAGE_DIR}", file=sys.stderr)
        sys.exit(1)
    count = 0
    for name in sorted(os.listdir(IMAGE_DIR)):
        if not name.lower().endswith(EXT):
            continue
        path = os.path.join(IMAGE_DIR, name)
        try:
            img = Image.open(path)
            if img.mode == "P":
                img = img.convert("RGBA")
            out = img.resize(TARGET_SIZE, Image.Resampling.LANCZOS)
            if path.lower().endswith((".jpg", ".jpeg")):
                out = out.convert("RGB")
                out.save(path, format="JPEG", quality=95)
            else:
                out.save(path, format="PNG")
            count += 1
            print(f"已处理: {name} -> 240x240")
        except Exception as e:
            print(f"失败 {name}: {e}", file=sys.stderr)
    print(f"共处理 {count} 张图片")


if __name__ == "__main__":
    main()
