"""Machine-source exclusions must not hide actual runtime reply literals."""
import unittest

from check_no_runtime_hard_reply import (
    is_machine_only_source_line, looks_machine_literal, looks_sentence_like, scan_diff,
)


class CompilerDiagnosticTests(unittest.TestCase):
    def test_complete_seatbelt_path_rules_are_machine_literals(self):
        for operation in ("read", "write"):
            rule = f'(deny file-{operation}* (subpath "{{path}}"))\n'
            self.assertTrue(looks_machine_literal(rule))
            self.assertFalse(looks_sentence_like(rule))

    def test_partial_rules_and_prose_never_gain_the_seatbelt_exemption(self):
        for value in [
            '(deny file-read* (subpath "{path}")',
            '(deny file-read* (subpath "{path}")) Please try again later.',
            'Please try again later. (deny file-read* (subpath "{path}"))',
            '(deny file-read* (subpath "{path}"))\nPlease try again later.',
        ]:
            self.assertFalse(looks_machine_literal(value))
            self.assertTrue(looks_sentence_like(value))

    def test_rule_on_the_same_line_does_not_hide_another_reply_literal(self):
        sample = (
            '+++ b/crates/clawd/src/process_sandbox_private.rs\n'
            '@@ -1,0 +1,1 @@\n'
            '+rules.push_str("(deny file-read* (subpath \\"{path}\\"))"); '
            'let reply = "Please try again later.";\n'
        )
        self.assertEqual(
            [candidate.literal for candidate in scan_diff(sample)],
            ["Please try again later."],
        )

    def test_only_complete_must_use_attributes_are_machine_only(self):
        for line in [
            '#[must_use = "retain ownership when reply delivery fails"]',
            '  #[ must_use = "retain the \\"attempted\\" owner" ]  ',
        ]:
            self.assertTrue(is_machine_only_source_line(line))

    def test_partial_attributes_and_appended_runtime_code_are_not_excluded(self):
        for line in [
            '#[must_use = "retain ownership when reply delivery fails"',
            '#[must_use = "retain ownership"]; let reply = "Please try again later.";',
            'let reply = "retain ownership when reply delivery fails";',
            'let reply = r#"#[must_use = something]"#;',
        ]:
            self.assertFalse(is_machine_only_source_line(line))

    def test_diff_scan_preserves_identical_prose_outside_compiler_attribute(self):
        text = "retain ownership when reply delivery fails"
        sample = (
            "+++ b/crates/clawd/src/local_process_owner_handshake.rs\n"
            "@@ -1,0 +1,3 @@\n"
            f'+#[must_use = "{text}"]\n'
            f'+let reply = "{text}";\n'
            '+#[must_use = "retain ownership"] let reply = "Please try again later.";\n'
        )
        self.assertEqual(
            [candidate.literal for candidate in scan_diff(sample)],
            [text, "Please try again later."],
        )


def run_tests():
    result = unittest.TextTestRunner().run(
        unittest.defaultTestLoader.loadTestsFromTestCase(CompilerDiagnosticTests)
    )
    if not result.wasSuccessful():
        raise AssertionError("compiler diagnostic boundary tests failed")


if __name__ == "__main__":
    run_tests()
