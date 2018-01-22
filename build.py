#!/usr/bin/env python2.7

import platform
import subprocess
import shutil
import os
import sys

plan_dir = os.path.abspath(os.path.join(__file__, "..", "src", "plan"))
PLANS = next(os.walk(plan_dir))[1]

os.chdir(os.path.abspath(os.path.join(__file__, "..")))

extra_features = ""
if len(sys.argv) > 1:
    extra_features = sys.argv[1]


def exec_and_redirect(args, env=None):
    print "[exec_and_redirect] {}".format(args)
    p = subprocess.Popen(args,
                         env=env)
    p.wait()
    if p.returncode != 0:
        exit(p.returncode)


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

for plan in PLANS:
    cmd = ["cargo",
           "+nightly",
           "build",
           "--no-default-features",
           "--features", " ".join([plan, extra_features])]
    exec_and_redirect(cmd)
    exec_and_redirect(cmd + ["--release"])
    shutil.copyfile("target/release/libmmtk{}".format(SUFFIX),
                    "./libmmtk{}".format(SUFFIX))

    if system == "Linux":
        exec_and_redirect(cmd + ["--target=i686-unknown-linux-gnu"])
        exec_and_redirect(
            cmd + ["--release", "--target=i686-unknown-linux-gnu"])
        shutil.copyfile(
            "target/i686-unknown-linux-gnu/release/libmmtk{}".format(SUFFIX),
            "./libmmtk_32{}".format(SUFFIX))

    exec_and_redirect([
        "clang",
        "-lmmtk",
        "-L.",
        "-Iapi",
        "-O3",
        "-o",
        "test_mmtk",
        "./api/main.c"])

    if system == "Linux":
        exec_and_redirect([
            "clang",
            "-lmmtk_32",
            "-L.",
            "-Iapi",
            "-O3", "-m32",
            "-o",
            "test_mmtk_32",
            "./api/main.c"])

    exec_and_redirect(["./test_mmtk"], env={LIBRARY_PATH: "."})
    os.remove("./test_mmtk")
    if system == "Linux":
        exec_and_redirect(["./test_mmtk_32"], env={LIBRARY_PATH: "."})
        os.remove("./test_mmtk_32")
