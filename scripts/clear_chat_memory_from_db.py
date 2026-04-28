#!/usr/bin/env python3
"""
Clear chat/session memory tables in the main RustClaw SQLite DB (default: data/rustclaw.db).

Removes:
  memories, long_term_memories, user_preferences,
  memory_retrieval_index (+ fts5 side table),
  conversation_states, clarify_states, followup_frames, observed_facts_states.

Does NOT remove: users, tasks, channel_bindings, auth_keys, scheduled_jobs, audit, webd logins, etc.

Usage (stop clawd first to avoid SQLITE_BUSY):
  python3 scripts/clear_chat_memory_from_db.py
  python3 scripts/clear_chat_memory_from_db.py /path/to/rustclaw.db

Env:
  RUSTCLAW_DB  — override default path (still overridden by argv[1])
"""
from __future__ import annotations

import os
import sqlite3
import sys


def main() -> int:
    default = os.path.join(os.path.dirname(os.path.dirname(__file__)), "data", "rustclaw.db")
    path = sys.argv[1] if len(sys.argv) > 1 else os.environ.get("RUSTCLAW_DB", default)
    if not os.path.isfile(path):
        print(f"error: database file not found: {path}", file=sys.stderr)
        return 2

    stmts = [
        "DELETE FROM memory_retrieval_index",
        "DELETE FROM memory_retrieval_index_fts",
        "DELETE FROM memories",
        "DELETE FROM long_term_memories",
        "DELETE FROM user_preferences",
        "DELETE FROM conversation_states",
        "DELETE FROM clarify_states",
        "DELETE FROM followup_frames",
        "DELETE FROM observed_facts_states",
    ]

    conn = sqlite3.connect(path, timeout=60.0)
    conn.execute("PRAGMA busy_timeout = 60000")
    conn.execute("BEGIN IMMEDIATE")
    try:
        for s in stmts:
            conn.execute(s)
        conn.commit()
    except Exception as e:
        conn.rollback()
        print(f"error: {e}", file=sys.stderr)
        return 1
    finally:
        try:
            conn.execute("PRAGMA wal_checkpoint(TRUNCATE)")
        except Exception:
            pass
        conn.close()

    print(f"cleared chat memory tables OK: {path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
