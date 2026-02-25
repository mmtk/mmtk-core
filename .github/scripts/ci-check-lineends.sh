#!/bin/bash

# This scripts is executed during CI style tests to check line ends.
# In this project, text files use UNIX line ends, and must have a newline character at the end of the file.
#
# Note that not having a newline character at the end of a file may have unexpected consequences.
# For example, when concatenating multiple files,
# the last line of a file will be joined with the first line of the next file.
# The same may happen when including files using `#include` or `include!` directives in C or Rust.
#
# You may also run this script directly to check line ends.
# It can also automatically fix the line ends for you if you add the -f option:
#
#   ./.github/scripts/ci-check-lineends.sh -f
#
# Other options will be forwarded to the check-lineends.sh script, too.

BAD_LINE_ENDS=0

find . -name 'target' -prune -o -type f -a '(' \
    -name '.gitignore' \
    -o -name '*.rs' \
    -o -name '*.h' \
    -o -name '*.yml' \
    -o -name '*.sh' \
    -o -name '*.toml' \
    -o -name '*.lock' \
    -o -name '*.py' \
    -o -name '*.bt' \
    -o -name '*.bt.fragment' \
    -o -name '*.md' \
    -o -name '*.html' \
    -o -name '*.css' \
    -o -name '*.js' \
    ')' -print | while read FILE; do
    if ! $(dirname $0)/check-lineends.sh "$@" $FIX_ARG "$FILE"; then
        BAD_LINE_ENDS=1
    fi
done

if [[ "$BAD_LINE_ENDS" -ne 0 ]]; then
    echo "ERROR: Some text files have non-unix line ends or do not have newline character at the end of file."
    exit 1
fi
