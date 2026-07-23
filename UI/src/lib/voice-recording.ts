export interface VoiceInputDeviceOption {
  deviceId: string;
  label: string;
}

const VOICE_RECORDER_MIME_TYPES = [
  "audio/webm;codecs=opus",
  "audio/webm",
  "audio/mp4",
  "audio/ogg;codecs=opus",
] as const;

export function voiceAudioTrackConstraints(deviceId = ""): MediaTrackConstraints {
  return {
    ...(deviceId.trim() ? { deviceId: { exact: deviceId.trim() } } : {}),
    channelCount: { ideal: 1 },
    echoCancellation: { ideal: true },
    noiseSuppression: { ideal: true },
    autoGainControl: { ideal: true },
  };
}

export function preferredVoiceRecorderMimeType(
  isTypeSupported?: (mimeType: string) => boolean,
): string | undefined {
  const supports =
    isTypeSupported ??
    (typeof MediaRecorder !== "undefined" && MediaRecorder.isTypeSupported
      ? MediaRecorder.isTypeSupported.bind(MediaRecorder)
      : undefined);
  return supports
    ? VOICE_RECORDER_MIME_TYPES.find((mimeType) => supports(mimeType))
    : undefined;
}

export function voiceRecorderOptions(mimeType?: string): MediaRecorderOptions {
  return {
    ...(mimeType ? { mimeType } : {}),
    audioBitsPerSecond: 64_000,
  };
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
