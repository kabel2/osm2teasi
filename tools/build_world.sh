#!/bin/bash
# Build a Teasi map for every region in regions.tsv and leave one zip per region
# behind, sorted into a folder per continent.
#
#   tools/build_world.sh <out dir> [region pattern]
#
# The maps are signed with --generic, so they are not tied to one device: the
# firmware binds such a file to whatever device opens it first.  The record key
# still comes from the first eight digits of TEASI_DEVICE, which covers every
# device of the same production batch -- see README.md "Device binding".
#
# Safe to interrupt and run again: a region whose zip is already there is
# skipped, and the downloaded OSM extract is deleted as soon as its maps are
# built.  The elevation tiles are kept and shared between regions.
#
# Environment:
#   TEASI_DEVICE      serial whose first eight digits pick the record key
#   TEASI_GEOS        libgeos_c.so, needed by four of the six compilers
#   LAND              land_polygons.shp (coastlines and the relief's land cover)
#   MAX_PBF_MB        skip extracts above this; the default is a third of the
#                     machine's memory.  The peak is about 2.7 times the size of
#                     a big extract (Germany: 4.9 GB -> 13.3 GB), more for small
#                     ones, where fixed costs dominate.  A country with a large
#                     area adds its elevation grid to the streets and the
#                     terrain (China: 9.9 GB).
#   REGION_TIMEOUT    seconds one region may take before it is given up,
#                     default 6 hours, far above any region that finishes
#   DATE              version date of the files, default today
#   ONLY              rebuild just these layers (comma separated, e.g. ONLY=ta)
#                     in the zips that are already there: the other files are
#                     kept, the README is written anew.  A zip whose layers
#                     already carry DATE is skipped, so this resumes too.
#
# The region list is generated: `teasi regions > tools/regions.tsv` fetches
# Geofabrik's index and maps it onto the firmware's country codes (see
# rust/src/regions.rs for what is left out and why).
set -u -o pipefail

out=${1:?usage: build_world.sh <out dir> [region pattern]}
want=${2:-}
here=$(cd "$(dirname "$0")" && pwd)
teasi=$here/../rust/target/release/teasi
list=$here/regions.tsv
max=${MAX_PBF_MB:-$(awk '/^MemTotal:/ { print int($2 / 1024 / 3) }' /proc/meminfo)}
date=${DATE:-$(date +%Y%m%d)}
land=${LAND:-}
base=https://download.geofabrik.de
only=${ONLY:-}
# the zip entries the rebuilt layers replace
drop=(); for l in ${only//,/ }; do drop+=("*_$l.v*"); done

[ -x "$teasi" ] || { echo "no teasi binary at $teasi -- cargo build --release first"; exit 1; }
[ -r "$list" ] || { echo "no region list at $list"; exit 1; }
[ -z "$land" ] || [ -r "$land" ] || { echo "LAND=$land is not readable"; exit 1; }
[ -n "$land" ] || echo "note: no LAND=, so no coastlines and no land cover in the relief"

work=$out/work
tiles=$out/dem_tiles
report=$out/report.tsv
mkdir -p "$work" "$tiles" || exit 1
[ -s "$report" ] || printf 'region\tcountry\tstatus\tseconds\tbytes\n' > "$report"

# ONLY mode: true when every rebuilt layer in the zip already has this date
fresh() {
    local l
    for l in ${only//,/ }; do
        unzip -Z1 "$1" | grep -q "_$l\.v" || continue
        unzip -Z1 "$1" | grep -qx ".*_$l\.v$date" || return 1
    done
}

# Continent -> folder name: australia-oceania becomes Australia-Oceania.
folder() { echo "$1" | sed -e 's/\b\(.\)/\u\1/g'; }

total=0 built=0 skipped=0 failed=0
while IFS=$'\t' read -r continent region code country prefix; do
    case $continent in \#*|"") continue;; esac
    [ -z "$want" ] || [[ $region == *$want* ]] || continue
    total=$((total + 1))

    name=$(basename "$region")
    dir=$out/$(folder "$continent")
    zip=$dir/$name.zip
    if [ -n "$only" ]; then
        # rebuilding layers needs the zip with the others
        have=$([ ! -e "$zip" ] || fresh "$zip" && echo y)
    else
        have=$([ -e "$zip" ] && echo y)
    fi
    if [ -n "$have" ]; then
        skipped=$((skipped + 1))
        continue
    fi
    shared=""
    case $code in \**) shared=" (shares country code ${code#\*} with other regions)";; esac
    code=${code#\*}

    pbf=$work/$name.osm.pbf
    poly=$work/$name.poly
    stage=$work/$name
    t0=$SECONDS
    echo "=== $region -> $zip$shared"

    size=$(curl -sIL --max-time 120 "$base/$region-latest.osm.pbf" 2>/dev/null |
           tr -d '\r' | awk 'tolower($1) == "content-length:" { v = $2 } END { print v }')
    if [ -z "$only" ] && [ -n "${size:-}" ] && [ "$size" -gt $((max * 1024 * 1024)) ]; then
        echo "    $((size / 1024 / 1024)) MB extract, over MAX_PBF_MB=$max -- left out"
        printf '%s\t%s\ttoo-big-%sMB\t0\t0\n' "$region" "$country" "$((size / 1024 / 1024))" >> "$report"
        failed=$((failed + 1))
        continue
    fi

    if ! curl -sL --max-time 7200 -o "$pbf" "$base/$region-latest.osm.pbf" ||
       ! curl -sL --max-time 300  -o "$poly" "$base/$region.poly"; then
        echo "    download failed"
        printf '%s\t%s\tdownload-failed\t%s\t0\n' "$region" "$country" "$((SECONDS - t0))" >> "$report"
        rm -f "$pbf" "$poly"; failed=$((failed + 1)); continue
    fi

    rm -rf "$stage"
    if ! timeout "${REGION_TIMEOUT:-21600}" "$teasi" all --generic --country="$code" --tiles="$tiles" \
            ${land:+--land="$land"} ${only:+--only="$only"} "$pbf" "$poly" "$stage" "$date" \
            > "$work/$name.log" 2>&1 ||
       { [ -n "$only" ] && ! unzip -q "$zip" -x README.txt "${drop[@]}" -d "$stage"; }; then
        echo "    build failed, see $work/$name.log"
        tail -n3 "$work/$name.log" | sed 's/^/      /'
        printf '%s\t%s\tbuild-failed\t%s\t0\n' "$region" "$country" "$((SECONDS - t0))" >> "$report"
        rm -f "$pbf" "$poly"; failed=$((failed + 1)); continue
    fi
    rm -f "$pbf" "$poly" "$stage/dem.bin"

    # one README per zip, so a stranger needs nothing but the zip
    dates=$(cd "$stage" && ls *.v* | sed 's/.*\.v//' | sort -u | paste -sd/)
    {
        echo "Teasi map: $country ($region, OpenStreetMap of $dates)"
        echo
        echo "Copy every *.v* file into BikeNav/Map/Countries/ on the device."
        echo "Confirm the USB connection on the display first, or the volume stays"
        echo "unreadable.  BikeNav/packages.xml only matters when this replaces one of"
        echo "the original maps (Denmark, Germany, Norway, Sweden): their entries still"
        echo "name the old files, and the device warns at startup until they point at"
        echo "the new ones -- \`teasi info <file>\` prints the size and md5 to put in."
        echo
        echo "The device loads one file per layer and country and keeps the one with"
        echo "the newest date, so delete an older map of $country before copying, or"
        echo "it may be ignored.  It also only loads countries that are licensed for"
        echo "it -- country code $code here."
        [ -z "$shared" ] || echo "NOTE: this region$shared, so only one of them at a time."
        echo
        echo "Built from $base/$region-latest.osm.pbf with"
        echo "https://github.com/kabel2/osm2teasi -- map data (c) OpenStreetMap"
        echo "contributors, ODbL 1.0, elevation from Copernicus DEM GLO-90."
        echo
        (cd "$stage" && for f in *.v*; do printf '%s  %s bytes  md5 %s\n' \
            "$f" "$(stat -c%s "$f")" "$(md5sum "$f" | cut -d' ' -f1)"; done)
    } > "$stage/README.txt"

    mkdir -p "$dir"
    # -0: the chart files are LZMA and JPEG inside, deflate would only cost time
    if ! (cd "$stage" && zip -q -0 -X "$zip.part" README.txt *.v*); then
        echo "    zip failed"
        printf '%s\t%s\tzip-failed\t%s\t0\n' "$region" "$country" "$((SECONDS - t0))" >> "$report"
        rm -f "$zip.part"; failed=$((failed + 1)); continue
    fi
    mv "$zip.part" "$zip"
    rm -rf "$stage"
    bytes=$(stat -c%s "$zip")
    echo "    $((bytes / 1024 / 1024)) MB in $((SECONDS - t0)) s"
    printf '%s\t%s\tok\t%s\t%s\n' "$region" "$country" "$((SECONDS - t0))" "$bytes" >> "$report"
    built=$((built + 1))
done < "$list"

echo
echo "$total regions: $built built, $skipped already there, $failed left out"
echo "report: $report"
