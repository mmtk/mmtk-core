#!/bin/bash

VERBOSITY=1
FIX=0

log() {
    LEVEL=$1
    shift
    if [[ "$VERBOSITY" -ge "$LEVEL" ]]; then
        echo "$@"
    fi
}

usage() {
    echo "USAGE: $0 [-qvfh] FILENAME"
    echo "    -q  Quiet mode"
    echo "    -v  Verbose mode"
    echo "    -f  Fix files with wrong line ends"
    echo "    -h  Print help and exit"
    echo
    echo "This script checks if the file FILENAME has proper line ends:"
    echo "  1. it uses UNIX line ends, and"
    echo "  2. it has a newline character at the end of the file"
    echo "If you add the -f option, it will try to fix the line ends if wrong."
}

while getopts "qvfh" ARG; do
    case "$ARG" in
        q) VERBOSITY=0;;
        v) VERBOSITY=2;;
        f) FIX=1;;
        h) usage; exit 0;;
        ?) usage; exit 1;;
    esac
done

shift $((OPTIND-1))

if [[ "$#" -ne 1 ]]; then log 0 "Need to specify a file."; usage; exit 1; fi

FILENAME="$1"

log 2 "Checking file: $FILENAME"

RESULT=$(dos2unix --info=dme "$FILENAME")

if [[ "$?" -ne 0 ]]; then
    log 0 "Failed to check file: $FILENAME"
    exit 1
fi

read DLE MLE LLE REST <<< $RESULT

log 2 "Dos line ends: $DLE"
log 2 "Mac line ends: $MLE"
log 2 "Last line end: $LLE"

GOOD=1

if [[ "$LLE" == "noeol" ]]; then
    log 1 "File does not end with newline:" $FILENAME
    GOOD=0
fi

if [[ "$DLE" -ne 0 ]] || [[ "$MLE" -ne 0 ]]; then
    log 1 "File contains non-UNIX line endings:" $FILENAME
    GOOD=0
fi

if [[ "$GOOD" -eq 1 ]]; then
    log 2 "OK: $FILENAME"
else
    if [[ "$FIX" -eq 1 ]]; then
        log 1 "Fixing $FILENAME"
        dos2unix --add-eol "$FILENAME"
    else
        exit 2
    fi
fi
