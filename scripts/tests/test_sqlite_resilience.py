#!/usr/bin/env python3
"""Exercise SQLite durability guarantees with isolated temporary databases."""

from __future__ import annotations

import hashlib
import multiprocessing
import os
import pathlib
import sqlite3
import tempfile
import threading
import unittest


def open_database(path: pathlib.Path) -> sqlite3.Connection:
    connection = sqlite3.connect(path, timeout=5.0)
    connection.execute("PRAGMA journal_mode=WAL")
    connection.execute("PRAGMA synchronous=NORMAL")
    connection.execute("PRAGMA busy_timeout=5000")
    return connection


def crash_after_commit(path: str) -> None:
    connection = open_database(pathlib.Path(path))
    connection.execute("INSERT INTO records(value) VALUES ('committed-before-crash')")
    connection.commit()
    os._exit(17)


def crash_before_commit(path: str) -> None:
    connection = open_database(pathlib.Path(path))
    connection.execute("BEGIN IMMEDIATE")
    connection.execute("INSERT INTO records(value) VALUES ('uncommitted-before-crash')")
    os._exit(18)


def database_digest(connection: sqlite3.Connection) -> str:
    rows = connection.execute("SELECT id, value FROM records ORDER BY id").fetchall()
    digest = hashlib.sha256()
    for row in rows:
        digest.update(f"{row[0]}\0{row[1]}\n".encode())
    return digest.hexdigest()


class SqliteResilienceTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tempdir = tempfile.TemporaryDirectory(prefix="runtime-sqlite-resilience-")
        self.root = pathlib.Path(self.tempdir.name)
        self.path = self.root / "runtime.db"
        connection = open_database(self.path)
        connection.execute(
            "CREATE TABLE records(id INTEGER PRIMARY KEY, value TEXT NOT NULL UNIQUE)"
        )
        connection.execute("INSERT INTO records(value) VALUES ('seed')")
        connection.commit()
        connection.close()

    def tearDown(self) -> None:
        self.tempdir.cleanup()

    def assert_integrity(self, path: pathlib.Path) -> sqlite3.Connection:
        connection = open_database(path)
        self.assertEqual(connection.execute("PRAGMA integrity_check").fetchone()[0], "ok")
        return connection

    def test_wal_recovers_committed_work_after_process_crash(self) -> None:
        process = multiprocessing.Process(target=crash_after_commit, args=(str(self.path),))
        process.start()
        process.join(10)
        self.assertEqual(process.exitcode, 17)

        connection = self.assert_integrity(self.path)
        values = [row[0] for row in connection.execute("SELECT value FROM records ORDER BY id")]
        self.assertEqual(values, ["seed", "committed-before-crash"])
        connection.close()

    def test_wal_discards_uncommitted_work_after_process_crash(self) -> None:
        process = multiprocessing.Process(target=crash_before_commit, args=(str(self.path),))
        process.start()
        process.join(10)
        self.assertEqual(process.exitcode, 18)

        connection = self.assert_integrity(self.path)
        values = [row[0] for row in connection.execute("SELECT value FROM records ORDER BY id")]
        self.assertEqual(values, ["seed"])
        connection.close()

    def test_concurrent_readers_observe_complete_commits(self) -> None:
        errors: list[str] = []
        stop = threading.Event()

        def reader() -> None:
            connection = open_database(self.path)
            last_count = 0
            while not stop.is_set():
                count = connection.execute("SELECT COUNT(*) FROM records").fetchone()[0]
                if count < last_count:
                    errors.append(f"row count moved backwards: {last_count} -> {count}")
                last_count = count
            connection.close()

        readers = [threading.Thread(target=reader) for _ in range(4)]
        for thread in readers:
            thread.start()
        writer = open_database(self.path)
        try:
            for index in range(200):
                writer.execute("INSERT INTO records(value) VALUES (?)", (f"row-{index}",))
                writer.commit()
        finally:
            writer.close()
            stop.set()
            for thread in readers:
                thread.join(5)
        self.assertEqual(errors, [])
        connection = self.assert_integrity(self.path)
        self.assertEqual(connection.execute("SELECT COUNT(*) FROM records").fetchone()[0], 201)
        connection.close()

    def test_online_backup_restores_identical_committed_rows(self) -> None:
        source = open_database(self.path)
        source.executemany(
            "INSERT INTO records(value) VALUES (?)",
            [(f"backup-row-{index}",) for index in range(100)],
        )
        source.commit()
        expected_digest = database_digest(source)
        backup_path = self.root / "backup.db"
        backup = sqlite3.connect(backup_path)
        source.backup(backup, pages=16)
        backup.close()
        source.close()

        restored = self.assert_integrity(backup_path)
        self.assertEqual(database_digest(restored), expected_digest)
        restored.close()


if __name__ == "__main__":
    unittest.main(verbosity=2)
