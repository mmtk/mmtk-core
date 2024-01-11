#!/usr/bin/env python3
from string import Template
from argparse import ArgumentParser
from pathlib import Path
import tempfile
import sys
import os


def get_args():
    parser = ArgumentParser()
    parser.add_argument("-b", "--bpftrace", type=str, default="bpftrace",
                        help="Path of the bpftrace executable")
    parser.add_argument("-m", "--mmtk", type=str, required=True,
                        help="Path of the MMTk binary")
    parser.add_argument("-H", "--harness", action="store_true",
                        help="Only collect data for the timing iteration (harness_begin/harness_end)")
    parser.add_argument("-p", "--print-script", action="store_true",
                        help="Print the content of the bpftrace script")
    parser.add_argument(
        "-f", "--format", choices=["text", "json"], default="text", help="bpftrace output format")
    parser.add_argument("tool", type=str, help="Name of the bpftrace tool")
    return parser.parse_args()


def main():
    args = get_args()
    here = Path(__file__).parent.resolve()
    bpftrace_script = here / f"{args.tool}.bt"
    if not bpftrace_script.exists():
        print(f"Tracing script {str(bpftrace_script)} not found.")
        sys.exit(1)
    mmtk_bin = Path(args.mmtk)
    if not mmtk_bin.exists():
        print(f"MMTk binary {str(mmtk_bin)} not found.")
        sys.exit(1)
    prologue_file = here / \
        ("prologue_with_harness.bt.fragment" if args.harness else "prologue_without_harness.bt.fragment")
    prologue = prologue_file.read_text()
    epilogue = (here / "epilogue.bt.fragment").read_text()
    template = Template(prologue + bpftrace_script.read_text() + epilogue)
    with tempfile.NamedTemporaryFile(mode="w+t") as tmp:
        content = template.safe_substitute(
            MMTK=mmtk_bin, TMP_FILE=tmp.name)
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
                           "--unsafe", "-f", args.format, tmp.name])


if __name__ == "__main__":
    main()
