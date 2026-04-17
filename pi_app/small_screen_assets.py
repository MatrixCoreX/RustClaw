import os

from small_screen_config import _pi_app_dir


def find_assets():
    return os.path.join(_pi_app_dir(), "assets")


def find_splash_image():
    path = os.path.join(_pi_app_dir(), "RustClaw480X320.png")
    return path if os.path.isfile(path) else None


def find_image_dir():
    return os.path.join(_pi_app_dir(), "image")


def list_gallery_images():
    ext = (".png", ".jpg", ".jpeg", ".gif", ".bmp", ".webp")
    path = find_image_dir()
    if not os.path.isdir(path):
        return []
    out = []
    for name in sorted(os.listdir(path)):
        if name.lower().endswith(ext):
            out.append(os.path.join(path, name))
    return out
