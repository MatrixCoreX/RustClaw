from dataclasses import dataclass
import tkinter as tk

from small_screen_messages import message_channel_display_name


ANIMATION_INTERVAL_MS = 70
ANIMATION_STEP_PX = 4


@dataclass(frozen=True)
class CompanionMessageState:
    task_id: str = ""
    time: str = ""
    channel: str = ""
    question: str = ""
    reply: str = ""

    @property
    def is_replying(self):
        return bool(self.reply)

    @property
    def is_waiting(self):
        return bool(self.question) and not self.reply


def _single_line(value):
    return " ".join(str(value or "").replace("\r", "\n").split())


def compact_companion_text(value, limit):
    text = _single_line(value)
    if limit <= 0 or len(text) <= limit:
        return text
    return text[: max(1, limit - 1)].rstrip() + "…"


def select_companion_message(messages):
    for item in messages if isinstance(messages, list) else []:
        if not isinstance(item, dict):
            continue
        question = _single_line(item.get("question") or item.get("text"))
        reply = _single_line(item.get("reply"))
        if not question and not reply:
            continue
        return CompanionMessageState(
            task_id=str(item.get("task_id") or "").strip(),
            time=str(item.get("time") or "").strip(),
            channel=str(item.get("channel") or "").strip(),
            question=question,
            reply=reply,
        )
    return CompanionMessageState()


class RobotDuckView:
    """Small, code-drawn companion view backed by the app's activity feed."""

    def __init__(self, parent, translate, color, lang_getter):
        self.parent = parent
        self._translate = translate
        self._color = color
        self._lang_getter = lang_getter
        self._state = CompanionMessageState()
        self._visible = False
        self._animation_job = None
        self._duck_x = 58
        self._direction = 1

        self._meta_var = tk.StringVar(value="")
        self._question_var = tk.StringVar(value="")
        self._speech_var = tk.StringVar(value="")

        self._meta_label = tk.Label(
            parent,
            textvariable=self._meta_var,
            font=("", 9),
            anchor="w",
            justify=tk.LEFT,
        )
        self._meta_label.pack(fill=tk.X, padx=12, pady=(4, 1))
        self._question_label = tk.Label(
            parent,
            textvariable=self._question_var,
            font=("", 10),
            anchor="w",
            justify=tk.LEFT,
        )
        self._question_label.pack(fill=tk.X, padx=12, pady=(0, 3))
        self._speech_label = tk.Label(
            parent,
            textvariable=self._speech_var,
            font=("", 11, "bold"),
            anchor="w",
            justify=tk.LEFT,
            wraplength=430,
            padx=10,
            pady=7,
        )
        self._speech_label.pack(fill=tk.X, padx=12)
        self._canvas = tk.Canvas(parent, height=108, highlightthickness=0, bd=0)
        self._canvas.pack(fill=tk.BOTH, expand=True, padx=8, pady=(2, 4))
        self.prepare([])

    def prepare(self, messages):
        self.parent.configure(bg=self._color("bg"))
        self._meta_label.configure(bg=self._color("bg"), fg=self._color("fg_dim"))
        self._question_label.configure(bg=self._color("bg"), fg=self._color("fg"))
        self._speech_label.configure(
            bg=self._color("box_bg"),
            fg=self._color("msg_agent_fg"),
            highlightbackground=self._color("box_border"),
            highlightcolor=self._color("box_border"),
            highlightthickness=1,
        )
        self._canvas.configure(bg=self._color("bg"))
        self.update_messages(messages)

    def show(self, messages):
        self._visible = True
        self.prepare(messages)
        self._sync_animation()

    def hide(self):
        self._visible = False
        self._cancel_animation()

    def update_messages(self, messages):
        self._state = select_companion_message(messages)
        lang = self._lang_getter()
        source = message_channel_display_name(self._state.channel, lang)
        if self._state.is_replying:
            status = self._translate("companion_replied")
            question = compact_companion_text(self._state.question, 72)
            speech = compact_companion_text(self._state.reply, 180)
        elif self._state.is_waiting:
            status = self._translate("companion_processing")
            question = compact_companion_text(self._state.question, 72)
            speech = self._translate("companion_waiting")
        else:
            source = ""
            status = self._translate("companion_ready")
            question = ""
            speech = self._translate("companion_idle")

        meta_parts = [part for part in (source, self._state.time, status) if part]
        self._meta_var.set(" · ".join(meta_parts))
        self._question_var.set(
            self._translate("companion_user_message").format(message=question)
            if question
            else ""
        )
        self._speech_var.set(speech)
        self._draw_scene()
        self._sync_animation()

    def _cancel_animation(self):
        if self._animation_job is None:
            return
        try:
            self.parent.after_cancel(self._animation_job)
        except tk.TclError:
            pass
        self._animation_job = None

    def _sync_animation(self):
        if not self._visible or self._state.is_replying:
            self._cancel_animation()
            return
        if self._animation_job is None:
            self._animation_job = self.parent.after(
                ANIMATION_INTERVAL_MS,
                self._animate,
            )

    def _animate(self):
        self._animation_job = None
        if not self._visible or self._state.is_replying:
            return
        width = max(420, self._canvas.winfo_width())
        left_bound = 54
        right_bound = width - 58
        next_x = self._duck_x + (ANIMATION_STEP_PX * self._direction)
        if next_x >= right_bound:
            next_x = right_bound
            self._direction = -1
        elif next_x <= left_bound:
            next_x = left_bound
            self._direction = 1
        self._duck_x = next_x
        self._draw_scene()
        self._sync_animation()

    def _draw_scene(self):
        canvas = self._canvas
        try:
            canvas.delete("all")
            width = max(420, canvas.winfo_width())
            ground_y = 91
            canvas.create_line(
                12,
                ground_y,
                width - 12,
                ground_y,
                fill=self._color("box_border"),
                width=2,
            )
            self._draw_duck(self._duck_x, 66, self._direction)
        except tk.TclError:
            pass

    def _draw_duck(self, x, y, direction):
        canvas = self._canvas
        outline = self._color("fg")
        panel = self._color("box_bg")
        led = self._color("status_ok")
        body = "#e2b13c"
        beak = "#e87532"
        metal = "#aeb8c5"

        canvas.create_oval(x - 31, y - 19, x + 25, y + 19, fill=body, outline=outline, width=2)
        canvas.create_rectangle(x - 18, y - 10, x + 8, y + 9, fill=panel, outline=outline, width=1)
        canvas.create_line(x - 12, y - 4, x + 2, y - 4, fill=led, width=2)
        canvas.create_line(x - 12, y + 2, x - 2, y + 2, fill=led, width=2)

        head_x = x + (25 * direction)
        canvas.create_oval(head_x - 17, y - 35, head_x + 17, y - 4, fill=metal, outline=outline, width=2)
        eye_x = head_x + (7 * direction)
        canvas.create_oval(eye_x - 3, y - 25, eye_x + 3, y - 19, fill=led, outline=outline)
        beak_base = head_x + (15 * direction)
        beak_tip = head_x + (30 * direction)
        canvas.create_polygon(
            beak_base,
            y - 18,
            beak_tip,
            y - 13,
            beak_base,
            y - 9,
            fill=beak,
            outline=outline,
        )
        canvas.create_line(head_x, y - 35, head_x, y - 44, fill=outline, width=2)
        canvas.create_oval(head_x - 3, y - 48, head_x + 3, y - 42, fill=led, outline=outline)

        for wheel_x in (x - 18, x + 13):
            canvas.create_oval(wheel_x - 8, y + 11, wheel_x + 8, y + 27, fill=metal, outline=outline, width=2)
            canvas.create_oval(wheel_x - 2, y + 17, wheel_x + 2, y + 21, fill=panel, outline=panel)
