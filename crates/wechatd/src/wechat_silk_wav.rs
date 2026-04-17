//! SILK → WAV (OpenClaw `silk-transcode.ts` alignment; sample rate 24kHz mono s16le).

/// Decode WeChat voice (SILK) to WAV; `None` if decoder rejects the buffer.
pub fn try_silk_to_wav(input: &[u8]) -> Option<Vec<u8>> {
    let pcm = silk_rs::decode_silk(input, 24_000).ok()?;
    if pcm.is_empty() {
        return None;
    }
    Some(wrap_pcm_i16le_mono_wav(&pcm, 24_000))
}

fn wrap_pcm_i16le_mono_wav(pcm: &[u8], sample_rate: u32) -> Vec<u8> {
    let pcm_bytes = pcm.len();
    let total_size = 44 + pcm_bytes;
    let mut buf = vec![0u8; total_size];
    let mut o = 0usize;
    buf[o..o + 4].copy_from_slice(b"RIFF");
    o += 4;
    buf[o..o + 4].copy_from_slice(&(total_size as u32 - 8).to_le_bytes());
    o += 4;
    buf[o..o + 4].copy_from_slice(b"WAVE");
    o += 4;
    buf[o..o + 4].copy_from_slice(b"fmt ");
    o += 4;
    buf[o..o + 4].copy_from_slice(&16u32.to_le_bytes());
    o += 4;
    buf[o..o + 2].copy_from_slice(&1u16.to_le_bytes());
    o += 2;
    buf[o..o + 2].copy_from_slice(&1u16.to_le_bytes());
    o += 2;
    buf[o..o + 4].copy_from_slice(&sample_rate.to_le_bytes());
    o += 4;
    buf[o..o + 4].copy_from_slice(&(sample_rate * 2).to_le_bytes());
    o += 4;
    buf[o..o + 2].copy_from_slice(&2u16.to_le_bytes());
    o += 2;
    buf[o..o + 2].copy_from_slice(&16u16.to_le_bytes());
    o += 2;
    buf[o..o + 4].copy_from_slice(b"data");
    o += 4;
    buf[o..o + 4].copy_from_slice(&(pcm_bytes as u32).to_le_bytes());
    o += 4;
    buf[o..o + pcm_bytes].copy_from_slice(pcm);
    buf
}
