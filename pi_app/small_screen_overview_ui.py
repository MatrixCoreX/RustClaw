import tkinter as tk

from small_screen_formatters import _line_clamp_text


def build_overview_layout(
    app,
    overview_us_stock_height,
    overview_a_stock_width,
    overview_market_height,
    overview_market_gap,
    overview_crypto_height,
    overview_runtime_height,
):
    body = getattr(app, "_overview_body", None)
    if body is None or not body.winfo_exists():
        return
    for child in body.winfo_children():
        child.destroy()
    us_stock_section = tk.Frame(
        body,
        bg=app._c("box_bg"),
        padx=8,
        pady=6,
        height=overview_us_stock_height,
    )
    us_stock_section.pack(fill=tk.X, pady=(2, 6))
    us_stock_section.pack_propagate(False)
    us_stock_title = tk.Label(
        us_stock_section,
        font=("", 10, "bold"),
        bg=app._c("box_bg"),
        fg=app._c("accent"),
        anchor="w",
    )
    us_stock_main_row = tk.Frame(us_stock_section, bg=app._c("box_bg"))
    us_stock_main_row.pack(fill=tk.X)
    us_stock_icon = tk.Label(
        us_stock_main_row,
        textvariable=app._overview_us_stock_icon_var,
        font=("DejaVu Sans", 20),
        bg=app._c("box_bg"),
        fg=app._c("accent"),
        width=0,
        anchor="w",
    )
    us_stock_icon.pack(side=tk.LEFT)
    us_stock_text_col = tk.Frame(us_stock_main_row, bg=app._c("box_bg"))
    us_stock_text_col.pack(side=tk.LEFT, fill=tk.X, expand=True, padx=(0, 0))
    us_stock_main = tk.Label(
        us_stock_text_col,
        textvariable=app._overview_us_stock_main_var,
        font=("", 12, "bold"),
        bg=app._c("box_bg"),
        fg=app._c("fg"),
        anchor="w",
        justify=tk.LEFT,
        wraplength=450,
    )
    us_stock_main.pack(anchor=tk.W)
    us_stock_meta = tk.Label(
        us_stock_text_col,
        textvariable=app._overview_us_stock_meta_var,
        font=("", 9),
        bg=app._c("box_bg"),
        fg=app._c("fg_dim"),
        anchor="w",
        justify=tk.LEFT,
        wraplength=450,
    )
    us_stock_meta.pack(anchor=tk.W, pady=(1, 0))
    us_stock_detail = tk.Label(
        us_stock_text_col,
        textvariable=app._overview_us_stock_detail_var,
        font=("", 8),
        bg=app._c("box_bg"),
        fg=app._c("fg_dim"),
        anchor="w",
        justify=tk.LEFT,
        wraplength=450,
    )
    us_stock_detail.pack(anchor=tk.W, pady=(2, 0))

    lower = tk.Frame(body, bg=app._c("bg"))
    lower.pack(fill=tk.BOTH, expand=True)
    left_col = tk.Frame(
        lower,
        bg=app._c("box_bg"),
        padx=8,
        pady=6,
        width=overview_a_stock_width,
        height=overview_market_height,
    )
    left_col.place(x=0, y=0, width=overview_a_stock_width, height=overview_market_height)
    left_col.pack_propagate(False)
    right_col = tk.Frame(lower, bg=app._c("bg"), padx=0, pady=0, height=overview_market_height)
    right_col.place(
        x=overview_a_stock_width + overview_market_gap,
        y=0,
        relwidth=1.0,
        width=-(overview_a_stock_width + overview_market_gap),
        height=overview_market_height,
    )
    right_top = tk.Frame(
        right_col,
        bg=app._c("box_bg"),
        padx=8,
        pady=6,
        height=overview_crypto_height,
    )
    right_top.place(x=0, y=0, relwidth=1.0, height=overview_crypto_height)
    right_top.pack_propagate(False)
    right_bottom = tk.Frame(
        right_col,
        bg=app._c("box_bg"),
        padx=8,
        pady=6,
        height=overview_runtime_height,
    )
    right_bottom.place(
        x=0,
        y=overview_market_height - overview_runtime_height,
        relwidth=1.0,
        height=overview_runtime_height,
    )
    right_bottom.pack_propagate(False)

    stock_title = tk.Label(left_col, textvariable=app._overview_stock_title_var, font=("", 10, "bold"), bg=app._c("box_bg"), fg=app._c("accent"), anchor="w")
    stock_value = tk.Label(left_col, textvariable=app._overview_stock_value_var, font=("", 11), bg=app._c("box_bg"), fg=app._c("fg"), anchor="w", justify=tk.LEFT, wraplength=215)
    stock_value.pack(anchor=tk.W)
    stock_meta = tk.Label(left_col, textvariable=app._overview_stock_meta_var, font=("", 8), bg=app._c("box_bg"), fg=app._c("fg_dim"), anchor="w", justify=tk.LEFT, wraplength=215)
    stock_meta.pack(anchor=tk.W, pady=(4, 0))

    crypto_title = tk.Label(right_top, textvariable=app._overview_crypto_title_var, font=("", 10, "bold"), bg=app._c("box_bg"), fg=app._c("accent"), anchor="w")
    crypto_value = tk.Label(right_top, textvariable=app._overview_crypto_value_var, font=("", 11), bg=app._c("box_bg"), fg=app._c("fg"), anchor="w", justify=tk.LEFT, wraplength=280)
    crypto_value.pack(anchor=tk.W)
    crypto_meta = tk.Label(right_top, textvariable=app._overview_crypto_meta_var, font=("", 8), bg=app._c("box_bg"), fg=app._c("fg_dim"), anchor="w", justify=tk.LEFT, wraplength=280)
    crypto_meta.pack(anchor=tk.W, pady=(4, 0))

    runtime_title = tk.Label(right_bottom, font=("", 9), bg=app._c("box_bg"), fg=app._c("fg_dim"), anchor="w")
    runtime_value = tk.Label(right_bottom, textvariable=app._overview_dashboard_value_var, font=("", 12, "bold"), bg=app._c("box_bg"), fg=app._c("fg"), anchor="w", justify=tk.LEFT)
    runtime_value.pack(anchor=tk.W, pady=(1, 0))
    runtime_meta = tk.Label(right_bottom, textvariable=app._overview_dashboard_meta_var, font=("", 8), bg=app._c("box_bg"), fg=app._c("fg_dim"), anchor="w", justify=tk.LEFT, wraplength=150)
    runtime_meta.pack(anchor=tk.W, pady=(2, 0))

    app._overview_us_stock_title_label = us_stock_title
    app._overview_stock_title_label = stock_title
    app._overview_crypto_title_label = crypto_title
    app._overview_runtime_title_label = runtime_title

    for widget in (us_stock_section, us_stock_title, us_stock_main_row, us_stock_icon, us_stock_text_col, us_stock_main, us_stock_meta, us_stock_detail):
        app._bind_overview_open(widget, "us_stock")
    for widget in (left_col, stock_title, stock_value, stock_meta):
        app._bind_overview_open(widget, "stock")
    for widget in (right_col, right_top, crypto_title, crypto_value, crypto_meta):
        app._bind_overview_open(widget, "crypto")
    for widget in (right_bottom, runtime_title, runtime_value, runtime_meta):
        app._bind_overview_open(widget, "dashboard")


def render_dashboard_overview(app):
    body = getattr(app, "_overview_body", None)
    if body is None or not body.winfo_exists():
        return
    if not getattr(app, "_overview_us_stock_title_label", None):
        app._build_overview_layout()
    if isinstance(getattr(app, "_weather_data", None), dict):
        app._overview_us_stock_icon_var.set(str(app._weather_data.get("icon") or "◌").strip() or "◌")
    else:
        app._overview_us_stock_icon_var.set("◌")
    us_lines = app._overview_compact_rows(app._overview_us_stock_display_lines(), per_row=2)
    app._overview_us_stock_main_var.set(_line_clamp_text(us_lines[0] if len(us_lines) > 0 else "--", ("", 11), 440, max_lines=1))
    app._overview_us_stock_meta_var.set(_line_clamp_text(us_lines[1] if len(us_lines) > 1 else "", ("", 11), 440, max_lines=1))
    app._overview_us_stock_detail_var.set(_line_clamp_text(us_lines[2] if len(us_lines) > 2 else "", ("", 11), 440, max_lines=1))
    stock_lines = app._overview_stock_display_lines()
    app._overview_stock_value_var.set(_line_clamp_text("\n".join(stock_lines[:2]), ("", 11), 230, max_lines=2))
    app._overview_stock_meta_var.set(_line_clamp_text("\n".join(stock_lines[2:4]), ("", 11), 230, max_lines=2))
    crypto_lines = app._overview_compact_rows(
        app._overview_crypto_display_lines(),
        per_row=app._overview_crypto_compact_per_row(),
    )
    app._overview_crypto_value_var.set(_line_clamp_text(crypto_lines[0] if len(crypto_lines) > 0 else "--", ("", 11), 300, max_lines=1))
    app._overview_crypto_meta_var.set(_line_clamp_text(crypto_lines[1] if len(crypto_lines) > 1 else "", ("", 8), 300, max_lines=1))
    dashboard_summary = app._overview_dashboard_summary()
    app._overview_dashboard_value_var.set(_line_clamp_text(dashboard_summary, ("", 12, "bold"), 150, max_lines=1))
    app._overview_dashboard_meta_var.set(_line_clamp_text(app._overview_dashboard_meta(), ("", 8), 150, max_lines=1))
    app._overview_us_stock_icon_var.set("")
    app._overview_us_stock_title_label.config(text="", bg=app._c("box_bg"), fg=app._c("accent"))
    app._overview_stock_title_var.set("")
    app._overview_stock_title_label.config(bg=app._c("box_bg"), fg=app._c("accent"))
    app._overview_crypto_title_var.set("")
    app._overview_crypto_title_label.config(bg=app._c("box_bg"), fg=app._c("accent"))
    app._overview_runtime_title_label.config(bg=app._c("box_bg"), fg=app._c("fg_dim"))
    app._schedule_overview_scroll()
