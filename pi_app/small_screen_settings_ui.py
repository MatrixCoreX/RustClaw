import tkinter as tk


def prepare_settings_view(app):
    app._settings_header_row.config(bg=app._c("bg"))
    app._settings_back_btn.config(
        text=app._t("back"),
        bg=app._c("button_bg"),
        fg=app._c("button_fg"),
        activebackground=app._c("button_active_bg"),
        activeforeground=app._c("fg"),
    )
    app._settings_content_frame.config(bg=app._c("bg"))
    app._settings_menu_frame.config(bg=app._c("bg"))
    app._settings_menu_language_btn.config(text="› " + app._t("language"), bg=app._c("bg"), fg=app._c("accent"), activebackground=app._c("bg"), activeforeground=app._c("fg"))
    app._settings_menu_theme_btn.config(text="› " + app._t("theme"), bg=app._c("bg"), fg=app._c("accent"), activebackground=app._c("bg"), activeforeground=app._c("fg"))
    app._settings_menu_pages_btn.config(text="› " + app._t("page_visibility"), bg=app._c("bg"), fg=app._c("accent"), activebackground=app._c("bg"), activeforeground=app._c("fg"))
    app._settings_menu_system_btn.config(text="› " + app._t("settings_system"), bg=app._c("bg"), fg=app._c("accent"), activebackground=app._c("bg"), activeforeground=app._c("fg"))
    app._settings_language_frame.config(bg=app._c("bg"))
    for btn in (app._settings_lang_en_btn, app._settings_lang_cn_btn, app._settings_theme_default_btn, app._settings_theme_matrix_btn, app._settings_show_messages_btn, app._settings_show_logs_btn, app._settings_show_gallery_btn, app._settings_show_skills_btn, app._settings_show_weather_btn, app._settings_show_stock_btn, app._settings_show_us_stock_btn, app._settings_show_crypto_btn):
        btn.config(
            bg=app._c("button_bg"),
            fg=app._c("button_fg"),
            activebackground=app._c("button_active_bg"),
            activeforeground=app._c("button_fg"),
            selectcolor=app._c("button_bg"),
        )
    app._settings_theme_frame.config(bg=app._c("bg"))
    app._settings_pages_frame.config(bg=app._c("bg"))
    app._settings_pages_row1.config(bg=app._c("bg"))
    app._settings_pages_row2.config(bg=app._c("bg"))
    app._settings_pages_row3.config(bg=app._c("bg"))
    app._settings_pages_row4.config(bg=app._c("bg"))
    app._settings_pages_row5.config(bg=app._c("bg"))
    app._settings_pages_row5_spacer.config(bg=app._c("bg"))
    app._settings_system_frame.config(bg=app._c("bg"))
    app._settings_wifi_btn.config(
        text=app._t("wifi_title"),
        bg=app._c("button_bg"),
        fg=app._c("button_fg"),
        activebackground=app._c("button_active_bg"),
        activeforeground=app._c("button_fg"),
    )
    app._settings_restart_btn.config(
        bg=app._c("button_bg"),
        fg=app._c("button_fg"),
        activebackground=app._c("button_active_bg"),
        activeforeground=app._c("button_fg"),
        disabledforeground=app._c("fg_dim"),
    )
    app._settings_reset_admin_btn.config(
        text=app._t("reset_admin_login") if app._settings_reset_admin_btn["state"] != tk.DISABLED else app._t("resetting_admin_login"),
        bg=app._c("button_bg"),
        fg=app._c("button_fg"),
        activebackground=app._c("button_active_bg"),
        activeforeground=app._c("button_fg"),
        disabledforeground=app._c("fg_dim"),
    )
    app._settings_reset_status_label.config(bg=app._c("bg"), fg=app._c("fg_dim"))
    try:
        app._settings_restart_btn.config(text=app._t("restart") if app._settings_restart_btn["state"] != tk.DISABLED else app._t("restarting"))
    except tk.TclError:
        pass
    app._settings_lang_var.set(app._lang)
    app._settings_theme_var.set(app._theme)
    app._settings_show_messages_var.set(app._show_messages_page)
    app._settings_show_logs_var.set(app._show_logs_page)
    app._settings_show_gallery_var.set(app._show_gallery_page)
    app._settings_show_skills_var.set(app._show_skills_page)
    app._settings_show_weather_var.set(app._show_weather_page)
    app._settings_show_stock_var.set(app._show_stock_page)
    app._settings_show_us_stock_var.set(app._show_us_stock_page)
    app._settings_show_crypto_var.set(app._show_crypto_page)
    app._refresh_settings_choice_labels()
    app._show_settings_menu()


def refresh_settings_choice_labels(app):
    messages_prefix = "● " if bool(app._settings_show_messages_var.get()) else "○ "
    logs_prefix = "● " if bool(app._settings_show_logs_var.get()) else "○ "
    gallery_prefix = "● " if bool(app._settings_show_gallery_var.get()) else "○ "
    skills_prefix = "● " if bool(app._settings_show_skills_var.get()) else "○ "
    weather_prefix = "● " if bool(app._settings_show_weather_var.get()) else "○ "
    stock_prefix = "● " if bool(app._settings_show_stock_var.get()) else "○ "
    us_stock_prefix = "● " if bool(app._settings_show_us_stock_var.get()) else "○ "
    crypto_prefix = "● " if bool(app._settings_show_crypto_var.get()) else "○ "
    app._settings_theme_default_btn.config(text=app._t("theme_default"))
    app._settings_theme_matrix_btn.config(text=app._t("theme_matrix"))
    app._settings_show_messages_btn.config(text=messages_prefix + app._t("show_messages_page"))
    app._settings_show_logs_btn.config(text=logs_prefix + app._t("show_logs_page"))
    app._settings_show_gallery_btn.config(text=gallery_prefix + app._t("show_nni_page"))
    app._settings_show_skills_btn.config(text=skills_prefix + app._t("show_skills_page"))
    app._settings_show_weather_btn.config(text=weather_prefix + app._t("show_weather_page"))
    app._settings_show_stock_btn.config(text=stock_prefix + app._t("show_stock_page"))
    app._settings_show_us_stock_btn.config(text=us_stock_prefix + app._t("show_us_stock_page"))
    app._settings_show_crypto_btn.config(text=crypto_prefix + app._t("show_crypto_page"))


def show_settings_category(app, category):
    frames = {
        "language": app._settings_language_frame,
        "theme": app._settings_theme_frame,
        "pages": app._settings_pages_frame,
        "system": app._settings_system_frame,
    }
    target = frames.get(category)
    if target is None:
        app._show_settings_menu()
        return
    app._settings_category = category
    for frame in (app._settings_menu_frame, app._settings_language_frame, app._settings_theme_frame, app._settings_pages_frame, app._settings_system_frame):
        if frame.winfo_manager():
            frame.pack_forget()
    if app._settings_header_row.winfo_manager() != "pack":
        app._settings_header_row.pack(fill=tk.X, pady=(6, 0))
    if app._settings_back_btn.winfo_manager() != "pack":
        app._settings_back_btn.pack(side=tk.RIGHT)
    target.pack(fill=tk.BOTH, expand=True)


def show_settings_menu(app):
    app._settings_category = "menu"
    for frame in (app._settings_language_frame, app._settings_theme_frame, app._settings_pages_frame, app._settings_system_frame):
        if frame.winfo_manager():
            frame.pack_forget()
    if app._settings_header_row.winfo_manager():
        app._settings_header_row.pack_forget()
    if app._settings_back_btn.winfo_manager():
        app._settings_back_btn.pack_forget()
    if app._settings_menu_frame.winfo_manager() != "pack":
        app._settings_menu_frame.pack(fill=tk.BOTH, expand=True)


def open_wifi_from_settings(app):
    app._switch_view("wifi")


def close_wifi_to_settings(app):
    app._switch_view("settings")
    app._show_settings_category("system")
