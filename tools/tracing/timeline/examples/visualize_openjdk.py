#!/usr/bin/env python3

# This is an example.  Read more details in ../EXTENSION.md

def enrich_event_extra(log_processor, name, ph, tid, ts, result, rest):
    match name:
        case "hello":
            result["args"] |= {
                "the_number": int(rest[0]),
            }

def enrich_meta_extra(log_processor, name, tid, ts, gc, wp, rest):
    if gc is not None:
        match name:
            case "hello2":
                gc["args"] |= {
                    "the_number": int(rest[0]),
                }

    if wp is not None:
        match name:
            case "hello3":
                wp["args"] |= {
                    "the_number": int(rest[0]),
                }
