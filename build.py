#!/usr/bin/env python2.7

import platform
import subprocess
import shutil
import os

plan_dir = os.path.abspath(os.path.join(__file__, "..", "src", "plan"))
PLANS = next(os.walk(plan_dir))[1]

os.chdir(os.path.abspath(os.path.join(__file__, "..")))


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
    cmd = ["cargo", "build", "--no-default-features", "--features", plan]
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
