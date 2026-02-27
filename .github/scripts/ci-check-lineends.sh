#!/bin/bash

# This is a driver script for check-lineends.py
# It finds text files in the project tree and checks/fixes its line ends.
#
# The CI runs this script during style checking.
#
# Developers may also run this script directly.
# It forwards all command line options to check-lineends.py.
# This means if you add the '-f' option,
# it will automatically fix the line ends of all files we concern.
#
#   ./.github/scripts/ci-check-lineends.sh -f
#
# In this project, text files use UNIX line ends, and must have a newline character at the end of the file.
# Note that not having a newline character at the end of a file may have unexpected consequences.
# For example, when concatenating multiple files,
# the last line of a file will be joined with the first line of the next file.
# The same may happen when including files using `#include` or `include!` directives in C or Rust.

BAD_LINE_ENDS=0

# TODO: When we introduce the '.gitattributes' file,
# make sure the patterns here matches the patterns in '.gitattributes'.
# Alternatively, find a way to automatically establish the list of files to check
# from the contents of '.gitattributes'.
FILES=$(find . -name 'target' -prune -o -type f -a '(' \
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
    -o -name 'COPYRIGHT' \
    -o -name 'LICENSE-*' \
    -o -name 'rust-toolchain' \
    -o -name '.gitignore' \
    ')' -print)

if ! xargs $(dirname $0)/check-lineends.py "$@" <<<$FILES; then
    BAD_LINE_ENDS=1
fi


if [[ "$BAD_LINE_ENDS" -ne 0 ]]; then
    echo "ERROR: Some text files have non-unix line ends or do not have newline character at the end of file."
    exit 1
fi
