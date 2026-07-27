import test from "node:test";
import assert from "node:assert/strict";

import {
  encodePcm16Wav,
  pcmWavRecordingSupported,
  shouldRetryVoiceCaptureWithDefault,
  voiceAudioTrackConstraints,
  voiceInputDeviceOptions,
} from "./voice-recording.ts";

test("builds speech-oriented audio constraints for the default microphone", () => {
  assert.deepEqual(voiceAudioTrackConstraints(), {
    channelCount: { ideal: 1 },
    sampleRate: { ideal: 48_000 },
    echoCancellation: { ideal: true },
    noiseSuppression: { ideal: true },
    autoGainControl: { ideal: false },
  });
});

test("binds an explicitly selected microphone without weakening speech constraints", () => {
  assert.deepEqual(voiceAudioTrackConstraints(" microphone-2 "), {
    deviceId: { exact: "microphone-2" },
    channelCount: { ideal: 1 },
    sampleRate: { ideal: 48_000 },
    echoCancellation: { ideal: true },
    noiseSuppression: { ideal: true },
    autoGainControl: { ideal: false },
  });
});

test("reports PCM recording unsupported outside a browser", () => {
  assert.equal(pcmWavRecordingSupported(), false);
});

test("encodes mono float samples as a standard little-endian PCM WAV", async () => {
  const blob = encodePcm16Wav(
    [new Float32Array([-1, -0.5, 0, 0.5, 1])],
    48_000,
  );
  const bytes = new Uint8Array(await blob.arrayBuffer());
  const view = new DataView(bytes.buffer);
  const ascii = (start: number, length: number) =>
    String.fromCharCode(...bytes.slice(start, start + length));

  assert.equal(blob.type, "audio/wav");
  assert.equal(ascii(0, 4), "RIFF");
  assert.equal(ascii(8, 4), "WAVE");
  assert.equal(ascii(12, 4), "fmt ");
  assert.equal(ascii(36, 4), "data");
  assert.equal(view.getUint16(20, true), 1);
  assert.equal(view.getUint16(22, true), 1);
  assert.equal(view.getUint32(24, true), 48_000);
  assert.equal(view.getUint16(34, true), 16);
  assert.equal(view.getUint32(40, true), 10);
  assert.deepEqual(
    Array.from({ length: 5 }, (_, index) => view.getInt16(44 + index * 2, true)),
    [-32768, -16384, 0, 16383, 32767],
  );
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
