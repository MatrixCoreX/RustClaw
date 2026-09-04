from dataclasses import dataclass
import time
import tkinter as tk
import tkinter.font as tkfont

from small_screen_messages import message_channel_display_name


ANIMATION_INTERVAL_MS = 70
ANIMATION_STEP_PX = 4
REPLY_PAUSE_SECONDS = 5.0
TEXT_WRAP_WIDTH = 420
DIALOGUE_LINE_BUDGET = 5


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


def wrap_companion_text(value, measure, max_width):
    text = _single_line(value)
    if not text:
        return []
    lines = []
    current = ""
    for char in text:
        candidate = current + char
        if not current or measure(candidate) <= max_width:
            current = candidate
            continue
        lines.append(current.rstrip())
        current = char.lstrip()
    if current:
        lines.append(current.rstrip())
    return lines


def fit_companion_text(value, measure, max_width, max_lines):
    lines = wrap_companion_text(value, measure, max_width)
    if max_lines <= 0 or len(lines) <= max_lines:
        return "\n".join(lines)
    visible = lines[:max_lines]
    tail = visible[-1].rstrip()
    while tail and measure(tail + "…") > max_width:
        tail = tail[:-1].rstrip()
    visible[-1] = (tail + "…") if tail else "…"
    return "\n".join(visible)


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


def companion_reply_identity(state):
    if not state.reply:
        return None
    message_id = state.task_id or "|".join((state.time, state.channel, state.question))
    return message_id, state.reply


def update_reply_pause(previous_identity, previous_until, state, now):
    reply_identity = companion_reply_identity(state)
    if reply_identity and reply_identity != previous_identity:
        return reply_identity, now + REPLY_PAUSE_SECONDS
    if not reply_identity:
        return None, 0.0
    return reply_identity, previous_until


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
        self._reply_identity = None
        self._reply_pause_until = 0.0

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
            wraplength=TEXT_WRAP_WIDTH,
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
        self._question_font = tkfont.Font(root=parent, font=self._question_label.cget("font"))
        self._speech_font = tkfont.Font(root=parent, font=self._speech_label.cget("font"))
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
            question, speech = self._fit_reply_dialogue(
                self._state.question,
                self._state.reply,
            )
        elif self._state.is_waiting:
            status = self._translate("companion_processing")
            question = fit_companion_text(
                self._translate("companion_user_message").format(
                    message=self._state.question
                ),
                self._question_font.measure,
                TEXT_WRAP_WIDTH,
                DIALOGUE_LINE_BUDGET - 1,
            )
            speech = self._translate("companion_waiting")
        else:
            source = ""
            status = self._translate("companion_ready")
            question = ""
            speech = self._translate("companion_idle")

        meta_parts = [part for part in (source, self._state.time, status) if part]
        self._meta_var.set(" · ".join(meta_parts))
        self._question_var.set(question)
        self._speech_var.set(speech)
        self._draw_scene()
        self._reply_identity, self._reply_pause_until = update_reply_pause(
            self._reply_identity,
            self._reply_pause_until,
            self._state,
            time.monotonic(),
        )
        self._sync_animation()

    def _fit_reply_dialogue(self, question, reply):
        question_text = (
            self._translate("companion_user_message").format(message=question)
            if question
            else ""
        )
        question_lines = wrap_companion_text(
            question_text,
            self._question_font.measure,
            TEXT_WRAP_WIDTH,
        )
        reply_lines = wrap_companion_text(
            reply,
            self._speech_font.measure,
            TEXT_WRAP_WIDTH,
        )
        question_budget = min(len(question_lines), 2)
        reply_budget = min(
            len(reply_lines),
            DIALOGUE_LINE_BUDGET - question_budget,
        )
        remaining = DIALOGUE_LINE_BUDGET - question_budget - reply_budget
        if remaining and len(question_lines) > question_budget:
            added = min(remaining, len(question_lines) - question_budget)
            question_budget += added
            remaining -= added
        if remaining and len(reply_lines) > reply_budget:
            reply_budget += min(remaining, len(reply_lines) - reply_budget)
        return (
            fit_companion_text(
                question_text,
                self._question_font.measure,
                TEXT_WRAP_WIDTH,
                question_budget,
            ),
            fit_companion_text(
                reply,
                self._speech_font.measure,
                TEXT_WRAP_WIDTH,
                reply_budget,
            ),
        )

    def _cancel_animation(self):
        if self._animation_job is None:
            return
        try:
            self.parent.after_cancel(self._animation_job)
        except tk.TclError:
            pass
        self._animation_job = None

    def _sync_animation(self):
        if not self._visible:
            self._cancel_animation()
            return
        pause_remaining = self._reply_pause_until - time.monotonic()
        if pause_remaining > 0:
            self._cancel_animation()
            self._animation_job = self.parent.after(
                max(1, int(pause_remaining * 1000) + 1),
                self._animate,
            )
            return
        if self._animation_job is None:
            self._animation_job = self.parent.after(
                ANIMATION_INTERVAL_MS,
                self._animate,
            )

    def _animate(self):
        self._animation_job = None
        if not self._visible:
            return
        if self._reply_pause_until > time.monotonic():
            self._sync_animation()
            return
        width = max(420, self._canvas.winfo_width())
        left_bound = 68
        right_bound = width - 68
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
        body = "#efc84a"
        head = "#f4d45b"
        beak = "#ffdc45"
        metal = "#aeb8c5"
        foot = "#e5a62e"

        tail_base = x - (28 * direction)
        tail_tip = x - (43 * direction)
        canvas.create_polygon(
            tail_base, y - 11,
            tail_tip, y - 20,
            tail_tip + (3 * direction), y - 5,
            tail_base, y + 2,
            fill=body,
            outline=outline,
        )
        canvas.create_oval(x - 32, y - 21, x + 29, y + 19, fill=body, outline=outline, width=2)

        wing_x = x - (5 * direction)
        canvas.create_oval(
            wing_x - 17,
            y - 12,
            wing_x + 14,
            y + 10,
            fill=head,
            outline=outline,
            width=1,
        )
        panel_x = x - (13 * direction)
        canvas.create_rectangle(panel_x - 8, y - 5, panel_x + 8, y + 6, fill=panel, outline=outline, width=1)
        canvas.create_line(panel_x - 4, y - 1, panel_x + 4, y - 1, fill=led, width=2)

        head_x = x + (28 * direction)
        canvas.create_oval(head_x - 18, y - 38, head_x + 18, y - 3, fill=head, outline=outline, width=2)
        eye_x = head_x + (7 * direction)
        canvas.create_oval(eye_x - 4, y - 28, eye_x + 4, y - 20, fill=panel, outline=outline)
        canvas.create_oval(eye_x - 1, y - 26, eye_x + 2, y - 23, fill=led, outline=led)

        beak_base = head_x + (15 * direction)
        beak_tip = head_x + (39 * direction)
        canvas.create_polygon(
            beak_base, y - 21,
            beak_tip, y - 19,
            beak_tip, y - 12,
            beak_base, y - 11,
            fill=beak,
            outline=outline,
            width=2,
        )
        canvas.create_line(beak_base, y - 16, beak_tip, y - 15, fill=outline, width=1)
        canvas.create_arc(
            head_x - 14,
            y - 34,
            head_x + 14,
            y - 7,
            start=80 if direction > 0 else 190,
            extent=100,
            style=tk.ARC,
            outline=metal,
            width=2,
        )
        canvas.create_line(head_x, y - 35, head_x, y - 44, fill=outline, width=2)
        canvas.create_oval(head_x - 3, y - 48, head_x + 3, y - 42, fill=led, outline=outline)

        for leg_x in (x - 18, x + 16):
            canvas.create_line(
                leg_x,
                y + 14,
                leg_x,
                y + 22,
                fill=metal,
                width=4,
            )
            toe_x = leg_x + (5 * direction)
            canvas.create_polygon(
                leg_x - 5,
                y + 20,
                toe_x + (10 * direction),
                y + 21,
                toe_x + (13 * direction),
                y + 26,
                leg_x - (7 * direction),
                y + 27,
                fill=foot,
                outline=outline,
                width=2,
            )
