#!/bin/bash
# CI-only native build: media tools must run on the supported oldest macOS,
# not merely on the newer machine supplying a Homebrew bottle.
set -euo pipefail
[[ "${GITHUB_ACTIONS:-}" == true && "$(uname -s)" == Darwin ]] || {
  echo 'macos_media_tools_require_native_ci' >&2; exit 64;
}
destination="${1:?destination required}"
[[ "$destination" == /* && ! -e "$destination" && ! -L "$destination" ]] || exit 64
minimum="${MACOSX_DEPLOYMENT_TARGET:?deployment target required}"
[[ "$minimum" =~ ^[0-9]+\.[0-9]+$ ]] || exit 64
jobs="$(sysctl -n hw.logicalcpu)"
work="$(mktemp -d "${RUNNER_TEMP:?}/macos-media.XXXXXX")"
trap 'rm -rf "$work"' EXIT
prefix="$work/install"
mkdir -p "$prefix/share/licenses"
export CFLAGS="-O2 -mmacosx-version-min=$minimum"
export CXXFLAGS="$CFLAGS"
export LDFLAGS="-mmacosx-version-min=$minimum"
export PKG_CONFIG_LIBDIR="$prefix/lib/pkgconfig"
export PKG_CONFIG_PATH="$PKG_CONFIG_LIBDIR"

fetch_source() {
  local name="$1" url="$2" commit="$3"
  git init -q "$work/$name"
  git -C "$work/$name" remote add origin "$url"
  git -C "$work/$name" fetch -q --depth=1 origin "$commit"
  git -C "$work/$name" checkout -q --detach FETCH_HEAD
  [[ "$(git -C "$work/$name" rev-parse HEAD)" == "$commit" ]]
}
fetch_source ffmpeg https://github.com/FFmpeg/FFmpeg.git 1041abdc962f4cc4f394aa8de9dc5236c0c3b9e7
fetch_source x264 https://code.videolan.org/videolan/x264.git b35605ace3ddf7c1a5d67a2eb553f034aef41d55
fetch_source dav1d https://github.com/videolan/dav1d.git b546257f770768b2c88258c533da38b91a06f737

(
  cd "$work/x264"
  ./configure --prefix="$prefix" --enable-static --disable-cli --disable-opencl
  make -j "$jobs"
  make install
)
meson setup "$work/dav1d-build" "$work/dav1d" --prefix="$prefix" --libdir=lib \
  --buildtype=release --default-library=static -Denable_tools=false -Denable_tests=false
meson compile -C "$work/dav1d-build" -j "$jobs"
meson install -C "$work/dav1d-build"

lame_archive="$work/lame.tar.gz"
curl --fail --location --retry 3 --output "$lame_archive" \
  https://downloads.sourceforge.net/project/lame/lame/4.0/lame-4.0.tar.gz
echo '3df5124d5ad3a98312ffd7ba6a9b36230e4f8a3e66d3ce0f425e336c32d216eb  '"$lame_archive" | shasum -a 256 -c -
mkdir "$work/lame"
tar -xzf "$lame_archive" -C "$work/lame" --strip-components=1
(
  cd "$work/lame"
  ./configure --prefix="$prefix" --disable-shared --enable-static --disable-frontend --disable-decoder --disable-nasm
  make -j "$jobs"
  make install
)
(
  cd "$work/ffmpeg"
  ./configure --prefix="$prefix" --disable-autodetect --disable-shared --enable-static \
    --disable-doc --disable-debug --disable-ffplay --enable-gpl \
    --enable-libx264 --enable-libmp3lame --enable-libdav1d --enable-zlib \
    --enable-bzlib --enable-iconv --enable-securetransport \
    --enable-videotoolbox --enable-audiotoolbox --pkg-config-flags=--static \
    --extra-libs=-liconv \
    --extra-cflags="-I$prefix/include -mmacosx-version-min=$minimum" \
    --extra-ldflags="-L$prefix/lib -mmacosx-version-min=$minimum"
  make -j "$jobs"
  make install
)
codesign --force --sign - "$prefix/bin/ffmpeg"
codesign --force --sign - "$prefix/bin/ffprobe"
for component in ffmpeg x264 dav1d lame; do
  mkdir -p "$prefix/share/licenses/$component"
  find "$work/$component" -maxdepth 1 -type f \( -name 'COPYING*' -o -name 'LICENSE*' \) \
    -exec cp '{}' "$prefix/share/licenses/$component/" \;
done
python3 - "$prefix" "$minimum" <<'PY'
import json
from pathlib import Path
import sys
root = Path(sys.argv[1])
(root / 'source-build.json').write_text(json.dumps({
    'schema_version': 1, 'minimum_macos_version': sys.argv[2],
    'ffmpeg': '1041abdc962f4cc4f394aa8de9dc5236c0c3b9e7',
    'x264': 'b35605ace3ddf7c1a5d67a2eb553f034aef41d55',
    'dav1d': 'b546257f770768b2c88258c533da38b91a06f737',
    'lame_sha256': '3df5124d5ad3a98312ffd7ba6a9b36230e4f8a3e66d3ce0f425e336c32d216eb',
}, sort_keys=True) + '\n')
PY
python3 - "$prefix" "$minimum" <<'PY'
from pathlib import Path
import sys
from scripts.bundle_macos_skill_dependencies import verify_media_tool_portability
for name in ('ffmpeg', 'ffprobe'):
    verify_media_tool_portability(Path(sys.argv[1]) / 'bin' / name, sys.argv[2])
PY
"$prefix/bin/ffmpeg" -hide_banner -loglevel error -nostdin \
  -f lavfi -i color=c=gray:s=64x64:r=5 -f lavfi -i sine=frequency=440:sample_rate=16000 \
  -t 0.6 -c:v libx264 -pix_fmt yuv420p -c:a aac "$work/fixture.mp4"
"$prefix/bin/ffmpeg" -hide_banner -loglevel error -nostdin \
  -i "$work/fixture.mp4" -vn -ar 16000 -ac 1 "$work/fixture.wav"
"$prefix/bin/ffmpeg" -hide_banner -loglevel error -nostdin \
  -i "$work/fixture.wav" -c:a libmp3lame "$work/fixture.mp3"
"$prefix/bin/ffprobe" -v error -show_entries stream=codec_name -of json "$work/fixture.mp3"
mkdir -p "$(dirname "$destination")"
mv "$prefix" "$destination"
