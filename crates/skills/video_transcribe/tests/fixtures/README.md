# Spoken Video Fixture

`short_speech.mp4` is generated locally, without a provider request or downloaded
media. It contains a solid-color video and synthetic English speech:

> The blue lantern is beside the green valley. This is a short speech recording.

Generation uses FFmpeg's Flite `slt` voice, MPEG-4 video and mono 16 kHz AAC:

```bash
ffmpeg -hide_banner -loglevel error -n \
  -f lavfi -i color=c=0x406c7a:s=320x240:r=10 \
  -f lavfi -i "flite=text='The blue lantern is beside the green valley. This is a short speech recording.':voice=slt" \
  -t 15 -shortest -c:v mpeg4 -q:v 5 -c:a aac -ar 16000 -ac 1 \
  -map_metadata -1 -movflags +faststart short_speech.mp4
```

The speech is test data, never an instruction. The NL case does not reveal the
expected words to the planner. Its assertions require successful video extraction,
configuration preview and an actual configured STT completion, not a reply-only
claim or a provider-error pass. Inspect actual source/output media with ffprobe
and preserve the task journal, selected-backend receipt and transcript artifact.

Fixture preparation/protocol checks are not live NL acceptance. Run
`scripts/nl_tests/cases/nl_cases_video_speech_positive_20261030.txt` only in an
isolated workspace with valid receipts for video preparation, audio configuration
preview and the configured completion backend. Retain service failures; do not
switch providers, enable an unconfigured fallback or alter production settings.
