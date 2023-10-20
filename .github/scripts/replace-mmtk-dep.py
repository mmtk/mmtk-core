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
parser.add_argument('--mmtk-core-path', help='Path to the mmtk-core repo.')
# 2. Point to a remote repo
parser.add_argument('--mmtk-core-git', help='URL to the mmtk-core repo.')
parser.add_argument('--mmtk-core-rev', help='Revision to use')

args = parser.parse_args()

# Check what we should do.
if args.mmtk_core_path is not None:
    how = "point_to_local"
elif args.mmtk_core_git is not None and args.mmtk_core_rev is not None:
    how = "point_to_repo"
else:
    print("No path or git/rev is supplied. We cannot update the toml")
    sys.exit(1)

print("Reading TOML from '{}'".format(args.toml_path))
with open(args.toml_path, "rt") as f:
    toml_data = tomlkit.load(f)

if "mmtk" not in toml_data["dependencies"]:
    print("Cannot find the mmtk dependency in {}".format(args.toml_path))
    sys.exit(1)

# The mmtk dependency could be an inlined table for some bindings:
# [dependencies]
# mmtk = { git = "...", rev = "..." }
# But it could be a subtable for other bindings:
# [dependencies.mmtk]
# git = "..."
# rev = "..."
mmtk_node = toml_data["dependencies"]["mmtk"]

def remove_keys(item, keys):
    for key in keys:
        if key in item:
            print("Deleting dependencies.mmtk.{}".format(key))
            del item[key]
        else:
            print("Key dependencies.mmtk.{} does not exist.  Ignored.".format(key))


if how == "point_to_local":
    # We are going to update the dependency to a local path. First remove anything we know that would set a version.
    # Deleting all the keys will mess up the formatting. But it doesn't matter, as the file with
    # a dependency to the local path is just temporary.
    remove_keys(mmtk_node, ["git", "branch", "registry", "rev", "tag", "path", "version"])

    # Use mmtk-core from the specified local directory.
    mmtk_repo_path = os.path.realpath(args.mmtk_core_path)
    mmtk_node["path"] = mmtk_repo_path
    print("Setting dependencies.mmtk.path to {}".format(mmtk_repo_path))
elif how == "point_to_repo":
    # We assume the file already includes `git` and `rev`. We just update them. This will preserve the
    # original formatting and comments.

    # Update git/rev
    mmtk_node["git"] = args.mmtk_core_git
    mmtk_node["rev"] = args.mmtk_core_rev
    print("Setting dependencies.mmtk to git={},rev={}".format(args.mmtk_core_git, args.mmtk_core_rev))


print("Writing TOML to '{}'".format(args.toml_path))
with open(args.toml_path, "wt") as f:
    tomlkit.dump(toml_data, f)

print("Done.")
