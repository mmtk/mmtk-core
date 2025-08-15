#!/usr/bin/env python3

import argparse
import gzip
import json
import re
import sys
from collections import defaultdict
from enum import Enum
from importlib.machinery import SourceFileLoader

RE_TYPE_ID = re.compile(r"\d+")
UNKNOWN_TYPE = "(unknown)"

class RootsKind(Enum):
    NORMAL = 0
    PINNING = 1
    TPINNING = 2

class Semantics(Enum):
    SOFT = 0
    WEAK = 1
    PHANTOM = 2

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

def begin_end_diff_dict(begin, end, begin_key="begin", end_key="end", diff_key="diff"):
    """ A convenient method for construct a dict of two values and their difference. """
    diff = end - begin
    return {begin_key: begin, end_key: end, diff_key: diff}

class LogProcessor:
    def __init__(self):
        self.type_id_name = {}
        self.results = []
        self.start_time = None
        self.current_gc = None
        self.current_work_packet = defaultdict(lambda: None)
        self.enrich_event_extra = None
        self.enrich_meta_extra = None

    def set_extra_handler(self, extra_handler):
        """
        Set a Python module `extra_handler` as the handler of unrecognized events. The
        `LogProcessor` will call the top-level functions ``enrich_event_extra`` and the
        ``enrich_meta_extra`` defined in `extra_handler` when encountering unrecognized non-meta and
        meta events, respectively.
        """
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
        """
        Process a line in the log that represents an event in the form of comma-separated values.
        """
        parts = line.split(",") # Split by comma.
        try:
            name, ph, tid, ts = parts[:4] # Extract the first four columns.
        except:
            print("Abnormal line: {}".format(line))
            raise
        tid = int(tid)
        ts = int(ts)
        args = parts[4:] # `args` will hold other columns.

        if not self.start_time:
            self.start_time = ts

        if ph == "meta":
            # Find the current GC and the current work packet, and call `enrich_meta`.
            # Will not generate any JSON object.
            gc = self.current_gc
            wp = self.current_work_packet[tid]
            self.enrich_meta(name, tid, ts, gc, wp, args)
        else:
            # Construct a JSON object with basic information, and call `enrich_event`.
            # The JSON object will be added to the results.
            result = {
                "name": name,
                "ph": ph,
                "tid": tid,
                # https://github.com/google/perfetto/issues/274
                "ts": (ts - self.start_time) / 1000.0,
                "args": {},
            }

            self.enrich_event(name, ph, tid, ts, result, args)

            self.results.append(result)

    def enrich_event(self, name, ph, tid, ts, result, args):
        """
        This function is called for every non-meta event the log processor encounters.

        `name`, `ph`, `tid` and `ts` are the first four columns in a comma-separated line in the
        output of `capture.py`.  `name` and `ph` are strings, while `tid` and `ts` integers. `args`
        is a list of strings that contains other columns.

        `result` is the JSON object (represented as a Python `dict`) that represents the event in
        the Trace Event Format.  This function can modify its contents for the concrete event.
        Specifically, ``result["args"]`` can be modified to display additional human-readable
        information in the Perfetto UI.

        Unrecognized events will be off-loaded to the ``enrich_event_extra`` function defined in the
        extension script (specified by ``-x`` on the command line).  It will be called with all
        parameters of this function, including `self` as the first argument.
        """

        match name:
            case "GC":
                # Put GC start/stop events in a virtual thread with tid=0
                result["tid"] = 0
                match ph:
                    case "B":
                        self.current_gc = result
                    case "E":
                        self.current_gc = None
            case "WORK":
                result["args"] |= {
                    "type_id": int(args[0]),
                }
                match ph:
                    case "B":
                        self.current_work_packet[tid] = result
                    case "E":
                        self.current_work_packet[tid] = None

            case "BUCKET_OPEN":
                result["args"] |= {
                    "stage": int(args[0]),
                }

            case _:
                if self.enrich_event_extra is not None:
                    # Call ``enrich_event_extra`` in the extension script if defined.
                    self.enrich_event_extra(self, name, ph, tid, ts, result, args)

    def enrich_meta(self, name, tid, ts, gc, wp, args):
        """
        This function is called for every meta event the log processor encounters.

        `name`, `tid` and `ts` are the first, third and fourth columns in a comma-separated line in
        the output of `capture.py`.  (The second column must be "meta" and is omitted.)  `name`is a
        string, while `tid` and `ts` integers. `args` is a list of strings that contains other
        columns after `ts`.

        `gc` and `wp` are the JSON objects (represented as Python `dict`) that represent the
        beginning of the current GC and the current work packet, respectively.  This function
        usually adds more contents to ``gc["args"]`` or ``wp["args"]`` to display additional
        human-readable information in the Perfetto UI.

        Unrecognized meta events will be off-loaded to the ``enrich_meta_extra`` function defined in
        the extension script (specified by ``-x`` on the command line).  It will be called with all
        parameters of this function, including `self` as the first argument.
        """

        processed_for_gc = True
        processed_for_wp = True

        # bpftrace may drop events.  Be conservative.
        if gc is not None:
            match name:
                case "gen_full_heap":
                    gc["args"] |= {
                        # Note: bool("0") == True
                        #       bool(int(0)) == bool(0) == False
                        "full_heap": bool(int(args[0])),
                    }

                case "immix_defrag":
                    gc["args"] |= {
                        "immix_is_defrag_gc": bool(int(args[0])),
                    }

                case _:
                    processed_for_gc = False
        else:
            processed_for_gc = False

        # bpftrace may drop events.  Be conservative.
        if wp is not None:
            match name:
                case "roots":
                    if "roots" not in wp["args"]:
                        wp["args"]["roots"] = []
                    roots_list = wp["args"]["roots"]
                    kind_id = int(args[0])
                    num = int(args[1])
                    match kind_id:
                        case RootsKind.NORMAL.value:
                            root_dict = {"kind": "normal_roots", "num_slots": num}
                        case RootsKind.PINNING.value:
                            root_dict = {"kind": "pinning_roots", "num_nodes": num}
                        case RootsKind.TPINNING.value:
                            root_dict = {"kind": "tpinning_roots", "num_nodes": num}

                    roots_list.append(root_dict)

                case "process_root_nodes":
                    wp["args"] |= {
                        "num_roots": int(args[0]),
                        "num_enqueued_nodes": int(args[1]),
                    }

                case "process_slots":
                    wp["args"] |= {
                        # Group args by "process_slots" and "scan_objects" because a ProcessEdgesWork
                        # work packet may do both if SCAN_OBJECTS_IMMEDIATELY is true.
                        "process_slots": {
                            "num_slots": int(args[0]),
                            "is_roots": int(args[1]),
                        },
                    }

                case "scan_objects":
                    total_scanned = int(args[0])
                    scan_and_trace = int(args[1])
                    scan_for_slots = total_scanned - scan_and_trace
                    wp["args"] |= {
                        # Put args in a group.  See comments in "process_slots".
                        "scan_objects": {
                            "total_scanned": total_scanned,
                            "scan_for_slots": scan_for_slots,
                            "scan_and_trace": scan_and_trace,
                        }
                    }

                case "sweep_chunk":
                    wp["args"] |= {
                        "allocated_blocks": int(args[0]),
                    }

                case "finalization":
                    wp["args"] |= {
                        "num_candidates": begin_end_diff_dict(int(args[0]), int(args[1])),
                        "ready_for_finalize": begin_end_diff_dict(int(args[2]), int(args[3])),
                    }

                case "reference_scanned":
                    semantics_int = int(args[0])
                    if semantics_int in Semantics:
                        semantics_str = Semantics(semantics_int).name
                    else:
                        semantics_str = "(Unknown)"
                    if "reference_scanned" not in wp["args"]:
                        wp["args"]["reference_scanned"] = []
                    wp["args"]["reference_scanned"].append({
                        "semantics": semantics_str,
                        "num_old": int(args[1]),
                        "num_new": int(args[2]),
                        "num_enqueued": int(args[3]),
                    })

                case _:
                    processed_for_wp = False
        else:
            processed_for_wp = False

        if not processed_for_gc and not processed_for_wp:
            # If we haven't touched an event, we offload it to the extension.
            if self.enrich_meta_extra is not None:
                # Call ``enrich_meta_extra`` in the extension script if defined.
                self.enrich_meta_extra(self, name, tid, ts, gc, wp, args)

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
        # Load the extension script.
        sfl = SourceFileLoader("extrahandler", args.extra)
        extra_handler_module = sfl.load_module()
        log_processor.set_extra_handler(extra_handler_module)

    log_processor.run(args.input)


if __name__ == '__main__':
    main()
