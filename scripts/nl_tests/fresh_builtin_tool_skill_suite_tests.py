"""Named replays do not invalidate a source suite's original prompt novelty."""
from pathlib import Path
import tempfile
import unittest
from unittest import mock

import check_fresh_builtin_tool_skill_suite as checker


class HistoricalNoveltyTests(unittest.TestCase):
    def fixture(self, root):
        cases = root / "cases"
        cases.mkdir()
        suite = cases / "fresh.txt"
        suite.write_text("fresh|unique_case|covers:fixture;requires_tool_call=true|Inspect fixture A\n")
        return cases, suite

    def history(self, root, cases, suite):
        with mock.patch.object(checker, "ROOT", root), mock.patch.object(checker, "CASES_ROOT", cases):
            return checker.historical_prompts(suite)

    def test_same_named_replay_and_verified_derived_chain_are_not_independent_cases(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            cases, suite = self.fixture(root)
            (cases / "replay.txt").write_text("replay|unique_case|replay|Inspect fixture A\n")
            (cases / "aggregate.txt").write_text("# source: cases/replay.txt\naggregate|worker_unique_case|aggregate|Inspect fixture A\n")
            (cases / "subset.txt").write_text("# source=cases/aggregate.txt\nsubset|worker_unique_case|subset|Inspect fixture A\n")
            self.assertEqual(self.history(root, cases, suite), {})

    def test_independent_duplicate_and_forged_source_prefix_still_fail_novelty(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            cases, suite = self.fixture(root)
            (cases / "independent.txt").write_text("other|different_case|test|Inspect fixture A\n")
            (cases / "forged.txt").write_text("other|worker_unique_case|aggregate|Inspect fixture A\n")
            history = self.history(root, cases, suite)
            self.assertEqual(len(history[checker.normalized_prompt("Inspect fixture A")]), 2)

    def test_missing_mismatched_outside_symlink_and_cyclic_sources_are_not_replay_proof(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            cases, suite = self.fixture(root)
            (root / "outside.txt").write_text("old|unique_case|fixture|Inspect fixture A\n")
            (cases / "linked.txt").symlink_to(root / "outside.txt")
            (cases / "different.txt").write_text("old|unique_case|fixture|Different request\n")
            sources = ["cases/absent.txt", "cases/different.txt", "outside.txt", "cases/linked.txt", "cases/cycle.txt"]
            for index, source in enumerate(sources):
                (cases / f"forged{index}.txt").write_text(f"# source: {source}\nother|worker_unique_case|aggregate|Inspect fixture A\n")
            (cases / "cycle.txt").write_text("# source: cases/cycle.txt\nother|worker_unique_case|aggregate|Inspect fixture A\n")
            history = self.history(root, cases, suite)
            self.assertEqual(len(history[checker.normalized_prompt("Inspect fixture A")]), 6)


if __name__ == "__main__":
    unittest.main()
