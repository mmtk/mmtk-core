#!/usr/bin/env python3

from argparse import ArgumentParser
from pathlib import Path
from string import Template
import os
import sys
import tempfile


def get_args():
    parser = ArgumentParser(
            description="""
This script is the first part of GC visualization.  It captures a trace,
recording the start and end of every GC and every work packet.
""",
            epilog="""
This script should be invoked as a normal user, but it will ask the user for
root password because it will use `sudo` to run `bpftrace`.  The user should
redirect the standard output to a log file so that the log file can be post-
processed by the `./visualize.py` script.
""")

    parser.add_argument("-b", "--bpftrace", type=str, default="bpftrace",
                        help="Path of the bpftrace executable")
    parser.add_argument("-m", "--mmtk", type=str, required=True,
                        help="Path of the MMTk binary")
    parser.add_argument("-H", "--harness", action="store_true",
                        help="Only collect data for the timing iteration (harness_begin/harness_end)")
    parser.add_argument("-p", "--print-script", action="store_true",
                        help="Print the content of the bpftrace script")
    parser.add_argument("-e", "--every", metavar="N", type=int, default=1,
                        help="Only capture every N-th GC"),
    return parser.parse_args()


def main():
    args = get_args()
    here = Path(__file__).parent.resolve()
    bpftrace_script = here / "capture.bt"
    mmtk_bin = Path(args.mmtk)

    if not mmtk_bin.exists():
        raise f"MMTk binary {str(mmtk_bin)} not found."

    template = Template(bpftrace_script.read_text())
    with tempfile.NamedTemporaryFile(mode="w+t") as tmp:
        content = template.safe_substitute(
            EVERY=args.every,
            HARNESS=int(args.harness),
            MMTK=mmtk_bin,
            TMP_FILE=tmp.name)
        if args.print_script:
            print(content)
        tmp.write(content)
        tmp.flush()
        # We use execvp to replace the current process instead of creating
        # a subprocess (or sh -c). This is so that when users invoke this from
        # the command line, Ctrl-C will be captured by bpftrace instead of the
        # outer Python script. The temporary file can then be cleaned up by
        # the END probe in bpftrace.
        #
        # In theory, you can implement this via pty, but it is very finicky
        # and doesn't work reliably.
        # See also https://github.com/anupli/running-ng/commit/b74e3a13f56dd97f73432d8a391e1d6cd9db8663
        os.execvp("sudo", ["sudo", args.bpftrace,
                           "--unsafe", tmp.name])


if __name__ == "__main__":
    main()
