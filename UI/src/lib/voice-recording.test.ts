import test from "node:test";
import assert from "node:assert/strict";

import {
  preferredVoiceRecorderMimeType,
  shouldRetryVoiceCaptureWithDefault,
  voiceAudioTrackConstraints,
  voiceInputDeviceOptions,
  voiceRecorderOptions,
} from "./voice-recording.ts";

test("builds speech-oriented audio constraints for the default microphone", () => {
  assert.deepEqual(voiceAudioTrackConstraints(), {
    channelCount: { ideal: 1 },
    echoCancellation: { ideal: true },
    noiseSuppression: { ideal: true },
    autoGainControl: { ideal: true },
  });
});

test("binds an explicitly selected microphone without weakening speech constraints", () => {
  assert.deepEqual(voiceAudioTrackConstraints(" microphone-2 "), {
    deviceId: { exact: "microphone-2" },
    channelCount: { ideal: 1 },
    echoCancellation: { ideal: true },
    noiseSuppression: { ideal: true },
    autoGainControl: { ideal: true },
  });
});

test("prefers Opus WebM and uses a voice-sized recorder bitrate", () => {
  const supported = new Set(["audio/webm;codecs=opus", "audio/mp4"]);
  const mimeType = preferredVoiceRecorderMimeType((candidate) => supported.has(candidate));

  assert.equal(mimeType, "audio/webm;codecs=opus");
  assert.deepEqual(voiceRecorderOptions(mimeType), {
    mimeType: "audio/webm;codecs=opus",
    audioBitsPerSecond: 64_000,
  });
});

test("keeps unique audio input devices only", () => {
  const options = voiceInputDeviceOptions([
    { kind: "videoinput", deviceId: "camera-1", label: "Camera" },
    { kind: "audioinput", deviceId: "microphone-1", label: " Built-in mic " },
    { kind: "audioinput", deviceId: "microphone-1", label: "Duplicate" },
    { kind: "audioinput", deviceId: "", label: "Unavailable" },
    { kind: "audioinput", deviceId: "microphone-2", label: "" },
  ]);

  assert.deepEqual(options, [
    { deviceId: "microphone-1", label: "Built-in mic" },
    { deviceId: "microphone-2", label: "" },
  ]);
});

test("retries only invalid device selection errors with the system default", () => {
  assert.equal(
    shouldRetryVoiceCaptureWithDefault(
      new DOMException("Selected microphone is unavailable", "NotFoundError"),
    ),
    true,
  );
  assert.equal(
    shouldRetryVoiceCaptureWithDefault(
      new DOMException("Microphone permission was denied", "NotAllowedError"),
    ),
    false,
  );
});
