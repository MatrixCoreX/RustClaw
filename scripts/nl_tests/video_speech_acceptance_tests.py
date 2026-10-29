"""Offline checks for the positive speech fixture; these are not live NL."""
import copy
import hashlib
import json
from pathlib import Path
import unittest

from manual_case_assertions import evaluate_expectations
from manual_case_assertions_tests import result_with_steps


ROOT = Path(__file__).resolve().parents[2]
SPEECH = "The blue lantern is beside the green valley. This is a short speech recording."


def observed(field, value):
    kind = "bool" if isinstance(value, bool) else "number" if isinstance(value, int) else "string"
    return {"field": field, "kind": kind,
            "excerpt": value if kind == "string" else json.dumps(value)}


def step(capability, skill, fields):
    return {"step_id": capability, "requested_action_type": "call_capability",
            "requested_capability": capability, "resolved_capability": capability,
            "executed_skill": skill, "status": "ok", "observed_evidence": {
                "items": [observed(field, value) for field, value in fields.items()]}}


class VideoSpeechAcceptanceTests(unittest.TestCase):
    def setUp(self):
        case = next(line for line in (ROOT / "scripts/nl_tests/cases/nl_cases_video_speech_positive_20261030.txt")
                    .read_text().splitlines() if line.startswith("video_speech|"))
        _, _, self.tags, self.prompt, expectation = case.split("|", 4)
        self.expectation = expectation.removeprefix("expect=")
        self.steps = [
            step("video.extract_audio", "video_transcribe", {
                "extra.action": "extract_audio", "extra.extracted_audio.sample_rate": 16000,
                "extra.extracted_audio.channels": 1, "extra.extracted_audio.deliver_to_user": False,
                "extra.extracted_audio.path": "/workspace/artifacts/video_audio.wav"}),
            step("audio.preview_transcribe", "audio_transcribe", {"extra.provider": "fixture-provider"}),
            step("audio.transcribe", "audio_transcribe", {"extra.text": SPEECH, "extra.provider": "fixture-provider"}),
        ]
        self.text = json.dumps({"text": SPEECH, "provider": "fixture-provider"})

    def accepted(self):
        obj = result_with_steps(self.steps, text=self.text)
        return evaluate_expectations(self.expectation, self.tags, obj, "succeeded", self.text,
                                     obj["data"]["result_json"], {})[0] == "pass"

    def test_real_fixture_hash_and_hidden_transcription_oracle(self):
        fixture = ROOT / "crates/skills/video_transcribe/tests/fixtures/short_speech.mp4"
        self.assertEqual(fixture.stat().st_size, 51100)
        with fixture.open("rb") as stream:
            self.assertEqual(hashlib.file_digest(stream, "sha256").hexdigest(),
                             "fbf05ecf7b5f7d2233739d3054d6fe57b41b76dee59e4d3568a64f693dc614c0")
        self.assertNotIn("blue lantern", self.prompt)
        self.assertNotIn("green valley", self.prompt)

    def test_configured_remote_completion_and_grounded_json(self):
        self.assertTrue(self.accepted())

    def test_configured_local_completion_is_allowed_without_forcing_a_provider(self):
        self.steps[2].update(requested_capability="local_asr.transcribe",
                             resolved_capability="local_asr.transcribe", executed_skill="local_asr")
        self.assertTrue(self.accepted())

    def test_missing_extraction_preview_or_completion_fails(self):
        original = copy.deepcopy(self.steps)
        for missing in range(3):
            with self.subTest(missing=missing):
                self.steps = [step for index, step in enumerate(original) if index != missing]
                self.assertFalse(self.accepted())

    def test_wrong_action_format_visibility_or_missing_path_fails(self):
        fields = {"extra.action": "other", "extra.extracted_audio.sample_rate": 8000,
                  "extra.extracted_audio.channels": 2, "extra.extracted_audio.deliver_to_user": True,
                  "extra.extracted_audio.path": None}
        original = copy.deepcopy(self.steps)
        for field, wrong in fields.items():
            with self.subTest(field=field):
                self.steps = copy.deepcopy(original)
                items = self.steps[0]["observed_evidence"]["items"]
                items[:] = [item for item in items if item["field"] != field]
                if wrong is not None:
                    items.append(observed(field, wrong))
                self.assertFalse(self.accepted())

    def test_provider_failure_is_not_a_positive_acceptance(self):
        self.steps[2]["status"] = "error"
        self.assertFalse(self.accepted())

    def test_reply_only_claims_and_wrong_words_or_provider_fail(self):
        for text in (SPEECH + " invented", "different words"):
            self.text = json.dumps({"text": text, "provider": "fixture-provider"})
            self.assertFalse(self.accepted())
        self.text = json.dumps({"text": SPEECH, "provider": "unobserved-provider"})
        self.assertFalse(self.accepted())

    def test_duplicate_extraction_and_unrequested_synthesis_fail(self):
        original = copy.deepcopy(self.steps)
        self.steps.append(copy.deepcopy(self.steps[0]))
        self.assertFalse(self.accepted())
        self.steps = original + [step("audio.synthesize", "audio_synthesize", {})]
        self.assertFalse(self.accepted())


if __name__ == "__main__":
    unittest.main()
