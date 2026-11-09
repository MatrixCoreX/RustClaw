"""Bound measurement fonts to their owning Tk interpreter, not each refresh."""
from collections import OrderedDict
import tkinter as tk
import tkinter.font as tkfont

MAX_MEASUREMENT_FONTS = 16
_CACHE_ATTRIBUTE = "_small_screen_measurement_fonts"


def clear_measurement_fonts(root):
    """Release layout-only fonts while keeping the interpreter's cleanup hook."""
    pool = getattr(root, _CACHE_ATTRIBUTE, None)
    if pool is not None:
        pool.clear()


def measurement_font(descriptor, root=None):
    root = root if root is not None else getattr(tk, "_default_root", None)
    # Named/mutable Font instances must continue to reflect live configuration.
    if root is None or not isinstance(descriptor, (tuple, list)):
        return tkfont.Font(root=root, font=descriptor)
    key = tuple(descriptor)
    try:
        hash(key)
    except TypeError:
        return tkfont.Font(root=root, font=descriptor)
    pool = getattr(root, _CACHE_ATTRIBUTE, None)
    if pool is None:
        pool = OrderedDict()
        setattr(root, _CACHE_ATTRIBUTE, pool)

        def on_destroy(event):
            if event.widget is root:
                pool.clear()

        root.bind("<Destroy>", on_destroy, add="+")
    font = pool.pop(key, None)
    if font is None:
        font = tkfont.Font(root=root, font=descriptor)
    pool[key] = font
    while len(pool) > MAX_MEASUREMENT_FONTS:
        pool.popitem(last=False)
    return font
