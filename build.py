#!/usr/bin/env python2.7

import platform
import subprocess
import shutil
import os


def exec_and_redirect(cmd, env=None):
    p = subprocess.Popen(cmd,
                         stdout=subprocess.PIPE,
                         stderr=subprocess.PIPE,
                         bufsize=1,  # line buffered
                         env=env)
    for line in p.stdout:
        print line.strip()
    for line in p.stderr:
        print line.strip()


system = platform.system()
assert system == "Darwin" or system == "Linux"

SUFFIX = ".so"
if system == "Darwin":
    SUFFIX = ".dylib"
elif system == "Linux":
    SUFFIX = ".so"

LIBRARY_PATH = "LD_LIBRARY_PATH"
if system == "Darwin":
    LIBRARY_PATH = "DYLD_LIBRARY_PATH"
elif system == "Linux":
    LIBRARY_PATH = "LD_LIBRARY_PATH"

exec_and_redirect(["cargo", "build"])
exec_and_redirect(["cargo", "build", "--release"])
shutil.copyfile("target/release/libmmtk{}".format(SUFFIX),
                "./libmmtk{}".format(SUFFIX))

if system == "Linux":
    exec_and_redirect(["cargo", "build", "--target=i686-unknown-linux-gnu"])
    exec_and_redirect(
        ["cargo", "build", "--release", "--target=i686-unknown-linux-gnu"])

exec_and_redirect([
    "clang",
    "-shared",
    "-fPIC",
    "-o", "libmmtkc{}".format(SUFFIX),
    "-O3",
    "bench/bump_allocator.c"])
exec_and_redirect([
    "clang",
    "-lmmtk",
    "-L.",
    "-Iapi",
    "-O3",
    "-o",
    "test_mmtk",
    "./api/main.c"])
exec_and_redirect([
    "clang",
    "-lmmtkc",
    "-L.",
    "-Iapi",
    "-O3",
    "-o",
    "test_mmtkc",
    "./api/main.c"])
exec_and_redirect(["./test_mmtk"], env={LIBRARY_PATH: "."})
exec_and_redirect(["./test_mmtkc"], env={LIBRARY_PATH: "."})
os.remove("./test_mmtk")
os.remove("./test_mmtkc")
