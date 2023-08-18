#!/bin/bash

echo "Replacing mmtk-core dependency for binding"

sudo apt-get update -y
sudo apt-get install python3-tomlkit
python $(dirname "$0")/replace-mmtk-dep.py "$@"
