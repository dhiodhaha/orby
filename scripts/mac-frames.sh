#!/bin/sh
# Measures Orby's frame pipeline on the Mac (docs/DEVELOPMENT.md, "macOS frame pipeline").
# Run on the Mac from the repo after `scripts/mac-bundle.sh release`:
#
#   scripts/mac-frames.sh            # starts the installed Orby on a test data folder, scans ~, runs the suite
#   scripts/mac-frames.sh suite      # the suite only, against an Orby already started this way
#
# It quits a running Orby, starts ~/Applications/Orby.app through LaunchServices (as a person starts
# it: an app started from ssh gets a lower priority main thread) with ORB_FRAMES=1 and its data in
# /tmp/orby-test (your settings, tasks and Disk cache are not touched), keeps the display awake for
# the run (caffeinate: a covered window still gets frames, an asleep display gives none) and floats
# the Disk window over other apps while it drives it. Each line of the result is one `frames-report`.
set -eu

log=/tmp/orby-test/orb-proto.log

t() {
    osascript -l JavaScript -e "ObjC.import(\"Foundation\"); \$.NSDistributedNotificationCenter.defaultCenter.postNotificationNameObjectUserInfoDeliverImmediately(\"dev.dhio.orby.test\", \"$1\", \$(), true)" >/dev/null
}

report() {
    t frames-report
    sleep 0.5
    echo "== $1"
    grep "frames" "$log" | tail -"${2:-4}" | sed "s/^[0-9-]* [0-9:]* //"
}

# measure <label> <drive> <seconds>
measure() {
    t frames-reset
    t "drive:$2"
    sleep "$3"
    sleep 0.5
    report "$1"
}

start() {
    pkill -x Orby 2>/dev/null || true
    sleep 1
    mkdir -p /tmp/orby-test
    rm -f "$log"
    open -n --env ORB_PROTO_DATA=/tmp/orby-test --env ORB_NO_FIRST_RUN=1 --env ORB_FRAMES=1 "$HOME/Applications/Orby.app"
    sleep 2
    t "disk-scan:$HOME"
    i=0
    while [ $i -lt 90 ] && ! grep -q "disk: scanned" "$log" 2>/dev/null; do sleep 1; i=$((i + 1)); done
    sleep 2
}

suite() {
    t float:disk:on
    t disk-tab:files
    sleep 1
    measure "Files, trackpad 120 Hz x 4 px" disk:trackpad:5:400,380 5
    measure "Files, trackpad 120 Hz x 12 px" disk:trackpad-fast:5:400,380 5
    measure "Files, trackpad 60 Hz x 8 px" disk:trackpad60:5:400,380 5
    measure "Files, wheel 30 notches a second" disk:wheel:5:400,380 5
    measure "Files, pointer 120 Hz" disk:hover:5:400,380 5
    measure "Files, trackpad 120 Hz x 4 px, the pointer on the list" disk:trackpad:5:400,380 5
    t disk-tab:folders
    sleep 1
    measure "Folders, trackpad 120 Hz x 4 px" disk:trackpad:5:500,380 5
    measure "Resize, a step at 60 Hz" disk:resize:4:400,380 4
    t frames-reset
    for _ in 1 2 3 4; do
        t disk-tab:files; sleep 0.4; t disk-tab:types; sleep 0.4; t disk-tab:folders; sleep 0.4
    done
    report "Tab switches"
    t frames-reset
    for _ in 1 2 3 4; do t disk-row:0; sleep 1.2; t disk-back; sleep 1.2; done
    report "Treemap reflow (in and back)"
    t disk-dupes
    sleep 1
    t disk-dupes-check
    i=0
    while [ $i -lt 120 ] && ! grep -q "disk: duplicates checked" "$log"; do sleep 1; i=$((i + 1)); done
    measure "Duplicates, trackpad 120 Hz x 4 px" disk:trackpad:5:400,380 5
    measure "Duplicates, pointer 120 Hz" disk:hover:5:400,380 5
    t frames-reset
    sleep 10
    report "Still for 10 s, the window open" 1
    t float:disk:off
    t disk-close
    # The Tasks window, with 40 tasks in the test data.
    t tasks-window
    sleep 1
    t float:tasks:on
    n=$(grep -c "task-add" "$log" || true)
    if [ "$n" -lt 40 ]; then
        for i in $(seq 1 40); do t "task-add:Frame test task $i"; done
    fi
    sleep 1
    measure "Tasks window, trackpad 120 Hz x 4 px" tasks:trackpad:5 5
    measure "Tasks window, pointer 120 Hz" tasks:hover:5 5
    t float:tasks:off
    t tasks-window-close
}

caffeinate -dimsu -t 1200 &
awake=$!
if [ "${1:-}" != suite ]; then
    start
fi
suite
kill $awake 2>/dev/null || true
