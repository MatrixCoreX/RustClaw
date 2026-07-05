import tkinter.font as tkfont


def _line_clamp_text(text, font, wraplength, max_lines=3, ellipsis="..."):
    content = str(text or "")
    if max_lines <= 0:
        return content
    measure_font = tkfont.Font(font=font)
    wrapped_lines = []
    for raw_line in content.split("\n"):
        if raw_line == "":
            wrapped_lines.append("")
            continue
        current = ""
        for ch in raw_line:
            candidate = current + ch
            if current and measure_font.measure(candidate) > wraplength:
                wrapped_lines.append(current)
                current = ch
            else:
                current = candidate
        wrapped_lines.append(current)
    if len(wrapped_lines) > max_lines:
        wrapped_lines = wrapped_lines[:max_lines]
        tail = wrapped_lines[-1].rstrip()
        while tail and measure_font.measure(tail + ellipsis) > wraplength:
            tail = tail[:-1]
        wrapped_lines[-1] = (tail + ellipsis) if tail else ellipsis
    if len(wrapped_lines) < max_lines:
        wrapped_lines.extend([""] * (max_lines - len(wrapped_lines)))
    return "\n".join(wrapped_lines)


def _strip_trailing_zeros(price_str):
    s = str(price_str).strip()
    if "." not in s:
        return s
    int_part, _, frac = s.partition(".")
    frac = frac.rstrip("0")
    return int_part if not frac else f"{int_part}.{frac}"


def _safe_float(value):
    try:
        return float(str(value).strip())
    except Exception:
        return None


def _fmt_signed_pct(current, prev_close):
    current_num = _safe_float(current)
    prev_num = _safe_float(prev_close)
    if current_num is None or prev_num is None or prev_num <= 0:
        return "--"
    pct = (current_num - prev_num) / prev_num * 100.0
    sign = "+" if pct >= 0 else ""
    return f"{sign}{pct:.2f}%"


def fmt_duration(sec):
    if sec is None or sec < 0:
        return "--"
    d = int(sec // 86400)
    h = int(sec // 3600)
    m = int((sec % 3600) // 60)
    s = int(sec % 60)
    if d > 0:
        return f"{d}d{h % 24}h"
    if h > 0:
        return f"{h}h{m}m"
    if m > 0:
        return f"{m}m{s}s"
    return f"{s}s"


def fmt_bytes(n):
    if n is None or n < 0:
        return "--"
    if n < 1024:
        return f"{n} B"
    if n < 1024 * 1024:
        return f"{n/1024:.1f} KB"
    return f"{n/(1024*1024):.1f} MB"
