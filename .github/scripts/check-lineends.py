#!/usr/bin/env python

import argparse
import re
import sys

ANY_NEWLINE = re.compile(rb'(\r\n|\r|\n)')

parser = argparse.ArgumentParser(
        prog='check-lineends.py',
        description='Line end checker for the MMTk project',
        epilog='''
This script checks if the file FILENAME has proper line ends:

1. it uses UNIX line ends, and
2. it has a newline character at the end of the file

If you add the -f option, it will try to fix the line ends if wrong.
''')

parser.add_argument('-q', '--quiet', action='store_true', help='Quiet mode')
parser.add_argument('-v', '--verbose', action='store_true', help='Verbose mode')
parser.add_argument('-f', '--fix', action='store_true', help='Fix files with wrong line ends')
parser.add_argument('filename', nargs='*', help='File name')

verbosity = 1

def pv(level, *args, **kwargs):
    if verbosity >= level:
        print(*args, **kwargs)

def process_file(filename, fix):
    pv(2, "Processing file:", filename)
    with open(filename, 'rb') as f:
        content = f.read()

    non_unix = b'\r' in content
    no_eol = not content.endswith(b'\n')
    wrong = non_unix or no_eol

    if non_unix:
        pv(1, "File contains non-UNIX line ends:", filename)
    if no_eol:
        pv(1, "File does not end with a newline character:", filename)

    if wrong and fix:
        pv(1, "Fixing file:", filename)
        fixed_content = ANY_NEWLINE.sub(b'\n', content)
        if no_eol:
            fixed_content += b'\n'
        with open(filename, 'wb') as f:
            f.write(fixed_content)

    return wrong


def main():
    args = parser.parse_args()

    global verbosity
    if args.quiet:
        verbosity = 0
    if args.verbose:
        verbosity = 2

    any_wrong = False

    for filename in args.filename:
        if process_file(filename, args.fix) == True:
            any_wrong = True

    if any_wrong:
        sys.exit(1)

if __name__=='__main__':
    main()
