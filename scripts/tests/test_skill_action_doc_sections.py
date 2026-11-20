import sys
from pathlib import Path
import unittest
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from skill_action_doc_sections import mark_actions, mark_parameters


class ActionDocumentationTests(unittest.TestCase):
    def test_action_boundaries_keep_shared_notes_outside_blocks(self):
        source = 'shared\n### `read`\nread body\n### `write`\nwrite body\n### Notes\nshared policy'
        result = mark_actions(source)
        self.assertIn('<!--a1:read-->\n### `read`\nread body\n<!--/a1-->', result)
        self.assertIn('<!--/a1-->\n### Notes\nshared policy', result)
        self.assertEqual(result.count('<!--a1:'), 2)

    def test_parameter_rows_group_by_exact_machine_action_and_preserve_common_rows(self):
        source = '| Action | Param |\n|---|---|\n| all | shared |\n| `read`, `write` | path |\n| `read`, `write` | mode |\n| `read` | range |'
        result = mark_parameters(source)
        self.assertIn('| all | shared |\n<!--a1:read,write-->', result)
        self.assertEqual(result.count('<!--a1:'), 2)
        self.assertEqual(result.count('<!--/a1-->'), 2)

    def test_undocumented_heading_grammar_is_not_guessed(self):
        source = '### Read files safely\npolicy\n### `read` (optional)\nshared notes'
        self.assertEqual(mark_actions(source), source)


if __name__ == '__main__':unittest.main()
