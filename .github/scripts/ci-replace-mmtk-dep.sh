#!/bin/bash

echo "Replacing mmtk-core dependency for binding"

apt-get update
apt-get install python-tomlkit
python $(dirname "$0")/replace-mmtk-dep.py "$@"
