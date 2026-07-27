export interface VoiceInputDeviceOption {
  deviceId: string;
  label: string;
}

export interface PcmWavRecordingSession {
  stop: () => Promise<Blob>;
  cancel: () => Promise<void>;
}

export type PcmWavRecordingErrorCode = "unsupported" | "empty";

export class PcmWavRecordingError extends Error {
  constructor(readonly code: PcmWavRecordingErrorCode) {
    super(`pcm_wav_recording_${code}`);
    this.name = "PcmWavRecordingError";
  }
}

type AudioContextConstructor = new (contextOptions?: AudioContextOptions) => AudioContext;

type AudioContextWindow = Window & {
  AudioContext?: AudioContextConstructor;
  webkitAudioContext?: AudioContextConstructor;
};

export function voiceAudioTrackConstraints(deviceId = ""): MediaTrackConstraints {
  return {
    ...(deviceId.trim() ? { deviceId: { exact: deviceId.trim() } } : {}),
    channelCount: { ideal: 1 },
    sampleRate: { ideal: 48_000 },
    echoCancellation: { ideal: true },
    noiseSuppression: { ideal: true },
    autoGainControl: { ideal: false },
  };
}

export function pcmWavRecordingSupported(): boolean {
  if (typeof window === "undefined") return false;
  const browserWindow = window as AudioContextWindow;
  return Boolean(browserWindow.AudioContext ?? browserWindow.webkitAudioContext);
}

export async function startPcmWavRecording(
  stream: MediaStream,
): Promise<PcmWavRecordingSession> {
  const browserWindow = window as AudioContextWindow;
  const AudioContextClass = browserWindow.AudioContext ?? browserWindow.webkitAudioContext;
  if (!AudioContextClass) throw new PcmWavRecordingError("unsupported");

  const context = new AudioContextClass({ latencyHint: "interactive" });
  const source = context.createMediaStreamSource(stream);
  const processor = context.createScriptProcessor(4096, 1, 1);
  const mutedOutput = context.createGain();
  const chunks: Float32Array[] = [];
  let stopped = false;
  let completedBlob: Blob | null = null;

  mutedOutput.gain.value = 0;
  processor.onaudioprocess = (event) => {
    if (stopped) return;
    chunks.push(new Float32Array(event.inputBuffer.getChannelData(0)));
  };
  source.connect(processor);
  processor.connect(mutedOutput);
  mutedOutput.connect(context.destination);
  if (context.state === "suspended") await context.resume();

  const cleanup = async () => {
    processor.onaudioprocess = null;
    source.disconnect();
    processor.disconnect();
    mutedOutput.disconnect();
    stream.getTracks().forEach((track) => track.stop());
    if (context.state !== "closed") {
      await context.close();
    }
  };

  return {
    stop: async () => {
      if (completedBlob) return completedBlob;
      stopped = true;
      await cleanup();
      if (!chunks.some((chunk) => chunk.length > 0)) {
        throw new PcmWavRecordingError("empty");
      }
      completedBlob = encodePcm16Wav(chunks, context.sampleRate);
      return completedBlob;
    },
    cancel: async () => {
      if (stopped) return;
      stopped = true;
      await cleanup();
    },
  };
}

export function encodePcm16Wav(chunks: readonly Float32Array[], sampleRate: number): Blob {
  const sampleCount = chunks.reduce((total, chunk) => total + chunk.length, 0);
  const bytesPerSample = 2;
  const dataByteLength = sampleCount * bytesPerSample;
  const buffer = new ArrayBuffer(44 + dataByteLength);
  const view = new DataView(buffer);

  writeAscii(view, 0, "RIFF");
  view.setUint32(4, 36 + dataByteLength, true);
  writeAscii(view, 8, "WAVE");
  writeAscii(view, 12, "fmt ");
  view.setUint32(16, 16, true);
  view.setUint16(20, 1, true);
  view.setUint16(22, 1, true);
  view.setUint32(24, sampleRate, true);
  view.setUint32(28, sampleRate * bytesPerSample, true);
  view.setUint16(32, bytesPerSample, true);
  view.setUint16(34, 16, true);
  writeAscii(view, 36, "data");
  view.setUint32(40, dataByteLength, true);

  let offset = 44;
  for (const chunk of chunks) {
    for (const rawSample of chunk) {
      const sample = Math.max(-1, Math.min(1, rawSample));
      view.setInt16(offset, sample < 0 ? sample * 0x8000 : sample * 0x7fff, true);
      offset += bytesPerSample;
    }
  }
  return new Blob([buffer], { type: "audio/wav" });
}

function writeAscii(view: DataView, offset: number, value: string): void {
  for (let index = 0; index < value.length; index += 1) {
    view.setUint8(offset + index, value.charCodeAt(index));
  }
}

export function voiceInputDeviceOptions(
  devices: ArrayLike<Pick<MediaDeviceInfo, "deviceId" | "kind" | "label">>,
): VoiceInputDeviceOption[] {
  const seen = new Set<string>();
  return Array.from(devices)
    .filter((device) => device.kind === "audioinput" && device.deviceId.trim())
    .filter((device) => {
      if (seen.has(device.deviceId)) return false;
      seen.add(device.deviceId);
      return true;
    })
    .map((device) => ({
      deviceId: device.deviceId,
      label: device.label.trim(),
    }));
}

export function shouldRetryVoiceCaptureWithDefault(error: unknown): boolean {
  if (!(error instanceof DOMException)) return false;
  return error.name === "OverconstrainedError" || error.name === "NotFoundError";
}
