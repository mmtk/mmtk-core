#!/usr/bin/env python3

import platform
import subprocess
import shutil
import os
import sys

MMTk_ROOT = os.path.join(__file__, "..", "..")

PLANS = []

# Find all plans from options.rs
options = ""
with open(os.path.abspath(os.path.join(MMTk_ROOT, "src", "util", "options.rs")), 'r') as file:
    options = file.read().replace('\n', '')
import re
search = re.search("enum PlanSelector \{([^\}]*)\}", options)
if search:
    raw_plans = search.group(1)
    PLANS = [x.strip() for x in raw_plans.split(",")]
else:
    print("cannot find PlanSelector in options.rs")
    sys.exit(1)

os.chdir(os.path.abspath(MMTk_ROOT))

extra_features = ""
if len(sys.argv) > 1:
    extra_features = sys.argv[1]


def exec_and_redirect(args, env=None):
    print("[exec_and_redirect] {} {}".format(args, env))
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

vmbinding = "vmbindings/dummyvm"

cmd = []
cmd.append("cargo")
cmd.extend([
    "build",
    "--manifest-path",
    "vmbindings/dummyvm/Cargo.toml",
    "--features", " ".join(extra_features)
])

exec_and_redirect(cmd)
exec_and_redirect(cmd + ["--release"])
shutil.copyfile("{}/target/release/libmmtk_dummyvm{}".format(vmbinding, SUFFIX),
                "./libmmtk{}".format(SUFFIX))

if system == "Linux":
    exec_and_redirect(cmd + ["--target=i686-unknown-linux-gnu"])
    exec_and_redirect(
        cmd + ["--release", "--target=i686-unknown-linux-gnu"])
    shutil.copyfile(
        "{}/target/i686-unknown-linux-gnu/release/libmmtk_dummyvm{}".format(vmbinding, SUFFIX),
        "./libmmtk_32{}".format(SUFFIX))

exec_and_redirect([
    "clang",
    "-lmmtk",
    "-L.",
    "-I{}/api".format(vmbinding),
    "-O3",
    "-o",
    "test_mmtk",
    "./examples/main.c"])

if system == "Linux":
    exec_and_redirect([
        "clang",
        "-lmmtk_32",
        "-L.",
        "-I{}/api".format(vmbinding),
        "-O3", "-m32",
        "-o",
        "test_mmtk_32",
        "./examples/main.c"])

for plan in PLANS:
    exec_and_redirect(["./test_mmtk"], env={LIBRARY_PATH: ".", "MMTK_PLAN": plan})
    if system == "Linux" and plan != 'PageProtect': # FIXME: PageProtect for 32bit MMTk does not work yet.
        exec_and_redirect(["./test_mmtk_32"], env={LIBRARY_PATH: ".", "MMTK_PLAN": plan})

os.remove("./test_mmtk")
if system == "Linux":
    os.remove("./test_mmtk_32")
