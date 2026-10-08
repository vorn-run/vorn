#!/bin/sh
# usage: apple/tools/rec.sh <outname> <harness args...>   (env passes through, e.g. VORN_SPIKE_DEMO)
cd "$(dirname "$0")/../.."
out=$1; shift
TMPDIR=/tmp timeout 90 ./target/release/spike-harness run "$@" --name rec-$out > target/rec-$out.log 2>&1 &
sleep 3
screencapture -x -v -V 13 -R 40,40,1280,760 target/rec-$out.mov
wait
ffmpeg -loglevel error -y -i target/rec-$out.mov -vf "scale=1280:-2,fps=30" -c:v libx264 -preset slow -crf 26 -pix_fmt yuv420p -movflags +faststart -an results/recordings/$out.mp4
rm -f target/rec-$out.mov results/raw/rec-$out.json
ls -la results/recordings/$out.mp4
