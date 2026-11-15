#!/usr/bin/env bash
# Steam library art in the game's own look (ui-spec.md: night stage, spotlight beams, ransom letters,
# the question-mark mascot), drawn from scratch with the bundled fonts: grid 600x900 and 920x430,
# hero 1920x620, logo, icon, into deck/art/.
set -euo pipefail
cd "$(dirname "$0")"
mkdir -p art
font=../assets/fonts/LilitaOne-Regular.ttf
body=../assets/fonts/AtkinsonHyperlegible-Bold.ttf
nix shell nixpkgs#imagemagick -c bash -s "$font" "$body" <<'SH'
set -euo pipefail
font=$1 body=$2
ink=#15112E beam=#2A2358 paper=#FFF2DA shade=#E5CFA8 teal=#14C8B4 mag=#F2297F mag7=#A8155A gold=#FFC53D
# letter blocks: background, letter colour, rotation
blocks=("$mag $paper -5" "$shade $mag7 3" "$ink $paper -2" "$teal $ink 4" "$paper $ink -3")
letter() { # letter size index -> letter.png
  read -r bg fg rot <<<"${blocks[$3 % 5]}"
  magick -background "$bg" -fill "$fg" -font "$font" -pointsize "$2" label:"$1" \
    -bordercolor "$bg" -border "$(( $2 / 10 ))" -background none -rotate "$rot" "l$3.png"
}
word() { # text size -> word.png
  local i=0 parts=()
  for c in $(echo "$1" | grep -o .); do letter "$c" "$2" "$i"; parts+=("l$i.png"); i=$((i + 1)); done
  magick "${parts[@]}" -background none -gravity center +smush "-$(( $2 / 25 ))" word.png
  rm -f "${parts[@]}"
}
mascot() { # size -> mascot.png: torn paper disc with a ? and two teal eyes, magenta offset shadow
  local s=$1 h=$(( $1 / 2 ))
  magick -size "${s}x${s}" xc:none \
    -fill "$mag" -draw "translate $((h + s / 20)),$((h + s / 20)) circle 0,0 $((h * 85 / 100)),0" \
    -fill "$paper" -draw "translate $h,$h circle 0,0 $((h * 85 / 100)),0" \
    -background none -wave "$(( s / 80 + 1 ))x$(( s / 12 ))" -gravity center -extent "${s}x${s}" \
    -fill "$ink" -font "$font" -pointsize "$(( s * 3 / 4 ))" -annotate +0+$((s / 30)) "?" \
    -fill "$teal" -draw "circle $((h - s / 14)),$((h - s / 7)) $((h - s / 14 + s / 28)),$((h - s / 7))" \
    -draw "circle $((h + s / 14)),$((h - s / 7)) $((h + s / 14 + s / 28)),$((h - s / 7))" mascot.png
}
stage() { # w h -> stage.png: night stage with two spotlight beams
  magick -size "${1}x${2}" "xc:$ink" -fill "$beam" \
    -draw "polygon $(( $1 * 30 / 100 )),0 $(( $1 * 45 / 100 )),0 $(( $1 * 62 / 100 )),$2 $(( $1 * 20 / 100 )),$2" \
    -draw "polygon $(( $1 * 80 / 100 )),0 $(( $1 * 92 / 100 )),0 $(( $1 * 70 / 100 )),$2 $(( $1 * 55 / 100 )),$2" stage.png
}
tag() { # w text -> tag.png: paper strip
  magick -background "$paper" -fill "$ink" -font "$body" -pointsize 26 -size "${1}x" -gravity center caption:"$2" \
    -bordercolor "$paper" -border 14 \( +clone -background "$shade" -shadow 100x0+6+7 \) +swap -background none -layers merge +repage tag.png
}

word HUNCH 150; mascot 300; stage 600 900; tag 460 "Party trivia. Phones are the buttons."
magick stage.png \( word.png -resize 540x \) -gravity north -geometry +0+150 -composite \
  mascot.png -gravity center -geometry +0+110 -composite tag.png -gravity south -geometry +0+90 -composite art/portrait.png
word HUNCH 120; mascot 260; stage 920 430
magick stage.png \( word.png -resize 560x \) -gravity west -geometry +40+0 -composite \
  mascot.png -gravity east -geometry +40+0 -composite art/wide.png
mascot 420; stage 1920 620  # no text: Steam overlays the logo
magick stage.png mascot.png -gravity east -geometry +260+0 -composite art/hero.png
word HUNCH 200; magick word.png -resize 1200x340 -background none -gravity center -extent 1200x340 art/logo.png
mascot 256; cp mascot.png art/icon.png
rm -f word.png mascot.png stage.png tag.png
SH
cat > art/logo.json <<'J'
{"nVersion":1,"logoPosition":{"pinnedPosition":"BottomLeft","nWidthPct":40,"nHeightPct":50}}
J
ls art
