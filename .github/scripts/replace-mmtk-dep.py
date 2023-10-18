#!/usr/bin/env python

import argparse
import os.path
import sys
import tomlkit

parser = argparse.ArgumentParser(
        description='Replace the mmtk-core dependency of a given VM binding',
        )

parser.add_argument('toml_path', help='Path to Cargo.toml')
# The following arguments are exclusive. Use either. If both are supplied, we use the local path.
# 1. Point to a local path
parser.add_argument('--mmtk_core_path', help='Path to the mmtk_core repo.')
# 2. Point to a remote repo
parser.add_argument('--mmtk_core_git', help='URL to the mmtk_core repo.')
parser.add_argument('--mmtk_core_rev', help='Revision to use')

args = parser.parse_args()

print("Reading TOML from '{}'".format(args.toml_path))
with open(args.toml_path, "rt") as f:
    toml_data = tomlkit.load(f)

if "mmtk" not in toml_data["dependencies"]:
    print("Cannot find the mmtk dependency in {}".format(args.toml_path))
    sys.exit(1)

# A new node for dependency
mmtk_node = tomlkit.inline_table()

# Construct the new mmtk node
if args.mmtk_core_path is not None:
    # Use mmtk-core from the specified local directory.
    mmtk_repo_path = os.path.realpath(args.mmtk_core_path)
    print("Setting dependencies.mmtk.path to {}".format(mmtk_repo_path))
    mmtk_node["path"] = mmtk_repo_path
elif args.mmtk_core_git is not None and args.mmtk_core_rev is not None:
    mmtk_node["git"] = args.mmtk_core_git
    mmtk_node["rev"] = args.mmtk_core_rev
else:
    print("No path or git/rev is supplied. We cannot update the toml")
    sys.exit(1)

# Store the mmtk node
toml_data["dependencies"]["mmtk"] = mmtk_node

print("Writing TOML to '{}'".format(args.toml_path))
with open(args.toml_path, "wt") as f:
    tomlkit.dump(toml_data, f)

print("Done.")
