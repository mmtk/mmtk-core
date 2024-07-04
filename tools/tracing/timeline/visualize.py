#!/usr/bin/env python3

import argparse
import gzip
import json
import re
import sys
from importlib.machinery import SourceFileLoader

RE_TYPE_ID = re.compile(r"\d+")
UNKNOWN_TYPE = "(unknown)"

def get_args():
    parser = argparse.ArgumentParser(
            description="""
This script is the second part of GC visualization.  It takes the output from
`./capture.py` as input, and format it into a JSON file suitable to be consumed
by Perfetto UI.
""")
    parser.add_argument("-x", "--extra", metavar="S", type=str,
                        help="path to extra log line handler")
    parser.add_argument("input", type=str, help="Input file"),
    return parser.parse_args()

class LogProcessor:
    def __init__(self):
        self.type_id_name = {}
        self.results = []
        self.start_time = None
        self.tid_current_work_packet = {}
        self.enrich_event_extra = None
        self.enrich_meta_extra = None

    def set_extra_handler(self, extra_handler):
        self.enrich_event_extra = getattr(extra_handler, "enrich_event_extra", None)
        self.enrich_meta_extra = getattr(extra_handler, "enrich_meta_extra", None)

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
            name, ph, tid, ts = parts[:4]
        except:
            print("Abnormal line: {}".format(line))
            raise
        tid = int(tid)
        ts = int(ts)
        rest = parts[4:]

        if not self.start_time:
            self.start_time = ts

        if ph == "meta":
            current = self.get_current_work_packet(tid)
            if current is not None:
                # eBPF may drop events.  Be conservative.
                self.enrich_meta(name, tid, ts, current, rest)
        else:
            result = {
                "name": name,
                "ph": ph,
                "tid": tid,
                # https://github.com/google/perfetto/issues/274
                "ts": (ts - self.start_time) / 1000.0,
                "args": {},
            }

            self.enrich_event(name, ph, tid, ts, result, rest)

            self.results.append(result)

    def enrich_event(self, name, ph, tid, ts, result, rest):
        match name:
            case "GC":
                # Put GC start/stop events in a virtual thread with tid=0
                result["tid"] = 0
            case "WORK":
                result["args"] |= {
                    "type_id": int(rest[0]),
                }
                match ph:
                    case "B":
                        self.set_current_work_packet(tid, result)
                    case "E":
                        self.clear_current_work_packet(tid, result)

            case "BUCKET_OPEN":
                result["args"] |= {
                    "stage": int(rest[0]),
                }

            case _:
                if self.enrich_event_extra is not None:
                    self.enrich_event_extra(self, name, ph, tid, ts, result, rest)

    def enrich_meta(self, name, tid, ts, current, rest):
        match name:
            case "roots":
                if "roots" not in current["args"]:
                    current["args"]["roots"] = []
                roots_list = current["args"]["roots"]
                kind_id = int(rest[0])
                num = int(rest[1])
                match kind_id:
                    case 0:
                        root_dict = {"kind": "normal_roots", "num_slots": num}
                    case 1:
                        root_dict = {"kind": "pinning_roots", "num_nodes": num}
                    case 2:
                        root_dict = {"kind": "tpinning_roots", "num_nodes": num}

                roots_list.append(root_dict)

            case "process_slots":
                current["args"] |= {
                    # Group args by "process_slots" and "scan_objects" because a ProcessEdgesWork
                    # work packet may do both if SCAN_OBJECTS_IMMEDIATELY is true.
                    "process_slots": {
                        "num_slots": int(rest[0]),
                        "is_roots": int(rest[1]),
                    },
                }

            case "scan_objects":
                total_scanned = int(rest[0])
                scan_and_trace = int(rest[1])
                scan_for_slots = total_scanned - scan_and_trace
                current["args"] |= {
                    # Put args in a group.  See comments in "process_slots".
                    "scan_objects": {
                        "total_scanned": total_scanned,
                        "scan_for_slots": scan_for_slots,
                        "scan_and_trace": scan_and_trace,
                    }
                }

            case "sweep_chunk":
                current["args"] |= {
                    "allocated_blocks": int(rest[0]),
                }

            case _:
                if self.enrich_meta_extra is not None:
                    self.enrich_meta_extra(self, name, tid, ts, current, rest)

    def set_current_work_packet(self, tid, result):
        self.tid_current_work_packet[tid] = result

    def get_current_work_packet(self, tid):
        return self.tid_current_work_packet[tid]

    def clear_current_work_packet(self, tid, result):
        self.tid_current_work_packet[tid] = None

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

    def run(self, input_file):
        print("Parsing lines...")
        with open(input_file) as f:
            start_time = None

            for line in f.readlines():
                line = line.strip()

                self.process_line(line)

        output_name = input_file + ".json.gz"

        print("Resolving work packet type names...")
        self.resolve_results()

        print(f"Dumping JSON output to {output_name}")
        with gzip.open(output_name, "wt") as f:
            self.output(f)


def main():
    args = get_args()

    log_processor = LogProcessor()

    if args.extra is not None:
        sfl = SourceFileLoader("extrahandler", args.extra)
        extra_handler_module = sfl.load_module()
        log_processor.set_extra_handler(extra_handler_module)

    log_processor.run(args.input)


if __name__ == '__main__':
    main()
