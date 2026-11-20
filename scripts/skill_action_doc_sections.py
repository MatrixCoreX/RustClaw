"""Versioned action boundaries for generated planner documentation."""
import re

OPEN = '<!--a1:'
CLOSE = '<!--/a1-->'
TOKEN = r'[a-z][a-z0-9_]*'


def action_tokens(value):
    value = value.strip()
    if not re.fullmatch(r'`?' + TOKEN + r'`?(?:\s*,\s*`?' + TOKEN + r'`?)*', value):
        return None
    tokens = tuple(re.findall(TOKEN, value))
    return None if any(x in ('all', 'any') for x in tokens) else tokens


def mark_actions(text):
    lines = []
    active = False
    for line in text.splitlines():
        if line.startswith('### '):
            if active:lines.append(CLOSE);active = False
            actions = action_tokens(line[4:])
            if actions:
                lines.append(OPEN + ','.join(actions) + '-->');active = True
        lines.append(line)
    if active:lines.append(CLOSE)
    return '\n'.join(lines)


def mark_parameters(text):
    lines = []
    previous = None
    for line in text.splitlines():
        actions = action_tokens(line.split('|')[1]) if line.startswith('|') and line.count('|') >= 2 else None
        if actions != previous:
            if previous:lines.append(CLOSE)
            if actions:lines.append(OPEN + ','.join(actions) + '-->')
            previous = actions
        lines.append(line)
    if previous:lines.append(CLOSE)
    return '\n'.join(lines)
