#!/bin/bash

VERBOSITY=1

usage() {
    echo "USAGE: $0 [-q] [-v] FILENAME"
}

while getopts "qvh" ARG; do
    case "$ARG" in
        q) VERBOSITY=0;;
        v) VERBOSITY=2;;
        h) usage; exit 0;;
        ?) usage; exit 1;;
    esac
done

shift $((OPTIND-1))

if [[ "$#" -ne 1 ]]; then echo "Need to specify a file."; usage; exit 1; fi

FILENAME="$1"

if [[ "$VERBOSITY" -ge 2 ]]; then
    echo "Checking file: $FILENAME"
fi

RESULT=$(dos2unix --info=dme "$FILENAME")

if [[ "$?" -ne 0 ]]; then
    echo "Failed to check file: $FILENAME"
    exit 1
fi

read DLE MLE LLE REST <<< $RESULT

if [[ "$VERBOSITY" -ge 2 ]]; then
    echo "Dos line ends: $DLE"
    echo "Mac line ends: $MLE"
    echo "Last line end: $LLE"
fi

GOOD=1

if [[ "$LLE" == "noeol" ]]; then
    if [[ "$VERBOSITY" -ge 1 ]]; then
        echo "File does not end with newline:" $FILENAME
    fi
    GOOD=0
fi

if [[ "$DLE" -ne 0 ]] || [[ "$MLE" -ne 0 ]]; then
    if [[ "$VERBOSITY" -ge 1 ]]; then
        echo "File contains non-UNIX line endings:" $FILENAME
    fi
    GOOD=0
fi

if [[ "$GOOD" -eq 1 ]]; then
    if [[ "$VERBOSITY" -ge 2 ]]; then
        echo "OK: $FILENAME"
    fi
else
    exit 2
fi
