#!/usr/bin/env python3

import argparse
import gzip
import json
import re
import sys

RE_TYPE_ID = re.compile(r"\d+")
UNKNOWN_TYPE = "(unknown)"

def get_args():
    parser = argparse.ArgumentParser(
            description="""
This script is the second part of GC visualization.  It takes the output from
`./capture.py` as input, and format it into a JSON file suitable to be consumed
by Perfetto UI.
""")
    parser.add_argument("input", type=str, help="Input file"),
    return parser.parse_args()

class LogProcessor:
    def __init__(self):
        self.type_id_name = {}
        self.results = []
        self.start_time = None

    def process_line(self, line):
        if line.startswith("@type_name"):
            self.process_type_line(line)
        elif "," in line:
            self.process_log_line(line)

    def process_type_line(self, line):
        left, right = line.split(":", 1)
        type_id = int(RE_TYPE_ID.search(left).group())
        type_name = right.strip()
        if type_name == "":
            # bpftrace sometimes sees empty strings when using the `str` function
            # See the "Known issues" section in README.md
            type_name = UNKNOWN_TYPE
        self.type_id_name[type_id] = type_name

    def process_log_line(self, line):
        parts = line.split(",")
        try:
            name, be, tid, ts = parts[:4]
        except:
            print("Abnormal line: {}".format(line))
            raise
        ts = int(ts)
        rest = parts[4:]

        if not self.start_time:
            self.start_time = ts

        result = {
            "name": name,
            "ph": be,
            "tid": tid,
            # https://github.com/google/perfetto/issues/274
            "ts": (ts - self.start_time) / 1000.0
        }

        match name:
            case "GC":
                # Put GC start/stop events in a virtual thread with tid=0
                result["tid"] = 0

            case "BUCKET_OPEN":
                result["args"] = {
                    "stage": int(rest[0])
                }

            case "INST":
                result["args"] = {
                    "val": int(rest[0])
                }

            case "WORK":
                result["args"] = {
                    "type_id": int(rest[0])
                }

        self.results.append(result)

    def resolve_results(self):
        for result in self.results:
            if result["name"] == "WORK":
                type_id = result["args"]["type_id"]
                type_name = self.type_id_name[type_id]
                if type_name == UNKNOWN_TYPE:
                    type_name = f"(unknown:{type_id})"
                result["name"] = type_name

    def output(self, outfile):
        json.dump({
            "traceEvents": self.results,
        }, outfile)


def main():
    args = get_args()

    log_processor = LogProcessor()

    print("Parsing lines...")
    with open(args.input) as f:
        start_time = None

        for line in f.readlines():
            line = line.strip()

            log_processor.process_line(line)

    output_name = args.input + ".json.gz"

    print("Resolving work packet type names...")
    log_processor.resolve_results()

    print(f"Dumping JSON output to {output_name}")
    with gzip.open(output_name, "wt") as f:
        log_processor.output(f)

if __name__ == '__main__':
    main()
